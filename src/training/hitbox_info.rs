//! "Hitbox Info": shows which hitbox of yours connected.
//!
//! The game says "one of your attacks connected", not which hitbox did it. To narrow it down
//! this looks at every hitbox that was active when the hit landed, then keeps the one(s) whose
//! damage matches what the opponent actually lost. A sweetspot and a sourspot differ in damage,
//! so this normally leaves a single hitbox.
//!
//!   - one match:      `#2 12.0%`
//!   - tied matches:   `#1/#2 8.0%` (hitboxes with the same damage cannot be told apart)
//!   - damage unknown: `#0/#1/#2?` (every hitbox that was active; shows when the hit did no
//!                     visible damage, e.g. to a shielding or invincible opponent)
//!
//! The full data (damage, size, angle, knockback) of every active hitbox, the damage dealt,
//! and the result are written to the mod's log file on each connect.
//!
//! The damage match assumes damage dealt scales all hitboxes of a move equally (staling,
//! charge), which holds for a single move. This does not identify the opponent's hurtbox.

use skyline::nn::ui2d::ResColor;
use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use crate::common::consts::{FighterId, OnOff};
use crate::common::{try_get_module_accessor, MENU};
use crate::hitbox_visualizer::ID_COLORS;
use crate::info;
use crate::training::info_box;
use training_mod_sync::*;

/// Hitbox IDs 0..8, the same range the Hitbox Visualization option covers
const HITBOX_IDS: i32 = 8;
/// Most IDs listed before the list is cut off with a "+"
const MAX_IDS_SHOWN: usize = 3;
/// Frames to wait after a hit for the opponent's damage to change before giving up
const DAMAGE_WAIT_FRAMES: u32 = 4;
/// Hitboxes whose damage is this close to the best match are kept as ties
const DAMAGE_TOLERANCE: f32 = 0.25;

const WHITE: ResColor = ResColor {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// A hit waiting for the opponent's damage to change
#[derive(Clone)]
struct Pending {
    /// (hitbox ID, damage) of every hitbox active when the hit landed
    hitboxes: Vec<(i32, f32)>,
    edge_frame: u32,
}

static FRAME: RwLock<u32> = RwLock::new(0);
static WAS_INFLICTING: RwLock<bool> = RwLock::new(false);
// The opponent's damage last frame, or negative before the first reading
static LAST_CPU_DAMAGE: RwLock<f32> = RwLock::new(-1.0);
// The most recent damage increase and the frame it happened on
static LAST_INCREASE: RwLock<f32> = RwLock::new(0.0);
static LAST_INCREASE_FRAME: RwLock<Option<u32>> = RwLock::new(None);
static PENDING: RwLock<Option<Pending>> = RwLock::new(None);

fn id_color(id: i32) -> ResColor {
    let c = ID_COLORS[(id as usize) % ID_COLORS.len()];
    ResColor {
        r: (c.x * 255.0) as u8,
        g: (c.y * 255.0) as u8,
        b: (c.z * 255.0) as u8,
        a: 255,
    }
}

fn format_ids(ids: &[i32]) -> String {
    let shown: Vec<String> = ids
        .iter()
        .take(MAX_IDS_SHOWN)
        .map(|id| format!("#{id}"))
        .collect();
    let more = if ids.len() > MAX_IDS_SHOWN { "+" } else { "" };
    format!("{}{more}", shown.join("/"))
}

/// The hitboxes whose damage best matches the damage dealt, plus any tied with the best
fn narrow(hitboxes: &[(i32, f32)], dealt: f32) -> Vec<(i32, f32)> {
    let best = hitboxes
        .iter()
        .map(|&(_, damage)| (damage - dealt).abs())
        .fold(f32::MAX, f32::min);
    hitboxes
        .iter()
        .copied()
        .filter(|&(_, damage)| (damage - dealt).abs() <= best + DAMAGE_TOLERANCE)
        .collect()
}

/// (message, color) for the notification
fn describe(hitboxes: &[(i32, f32)], dealt: Option<f32>) -> (String, ResColor) {
    match dealt {
        Some(dealt) => {
            let chosen = narrow(hitboxes, dealt);
            let ids: Vec<i32> = chosen.iter().map(|&(id, _)| id).collect();
            let message = format!("{} {:.1}%", format_ids(&ids), chosen[0].1);
            let color = if let [only] = ids[..] {
                id_color(only)
            } else {
                WHITE
            };
            (message, color)
        }
        None => {
            let ids: Vec<i32> = hitboxes.iter().map(|&(id, _)| id).collect();
            (format!("{}?", format_ids(&ids)), WHITE)
        }
    }
}

unsafe fn finish(pending: &Pending, dealt: Option<f32>) {
    let (message, color) = describe(&pending.hitboxes, dealt);
    info!(
        "Hitbox hit: active {:?} (id, damage), damage dealt {:?} -> {}",
        pending.hitboxes, dealt, message
    );
    info_box::show_hitbox(message, color);
}

/// Runs once per frame, from the player's update.
pub unsafe fn once_per_frame(module_accessor: &mut BattleObjectModuleAccessor) {
    if read(&MENU).hitbox_info != OnOff::ON {
        return;
    }
    let entry_id = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry_id != FighterId::Player as i32 {
        return;
    }

    let frame = read(&FRAME) + 1;
    assign(&FRAME, frame);

    // Track the opponent's damage so a hit can be matched to the damage it did
    if let Some(cpu) = try_get_module_accessor(FighterId::CPU) {
        let damage = DamageModule::damage(cpu, 0);
        let previous = read(&LAST_CPU_DAMAGE);
        if previous >= 0.0 && damage > previous + 0.001 {
            assign(&LAST_INCREASE, damage - previous);
            assign(&LAST_INCREASE_FRAME, Some(frame));
        }
        assign(&LAST_CPU_DAMAGE, damage);
    }

    // A new hit: remember which hitboxes were active
    let inflicting = AttackModule::is_infliction(module_accessor, *COLLISION_KIND_MASK_HIT);
    let was_inflicting = read(&WAS_INFLICTING);
    assign(&WAS_INFLICTING, inflicting);
    if inflicting && !was_inflicting {
        let hitboxes: Vec<(i32, f32)> = (0..HITBOX_IDS)
            .filter_map(|id| {
                if !AttackModule::is_attack(module_accessor, id, false) {
                    return None;
                }
                let damage = (*AttackModule::attack_data(module_accessor, id, false)).power;
                Some((id, damage))
            })
            .collect();
        if !hitboxes.is_empty() {
            assign(
                &PENDING,
                Some(Pending {
                    hitboxes,
                    edge_frame: frame,
                }),
            );
        }
    }

    // Resolve a pending hit once the damage shows up, or give up after a few frames.
    // Damage can land a frame before the game reports the hit, hence the `- 1`.
    if let Some(pending) = read_clone(&PENDING) {
        let damaged = read(&LAST_INCREASE_FRAME)
            .filter(|&increase_frame| increase_frame + 1 >= pending.edge_frame);
        if damaged.is_some() {
            assign(&PENDING, None);
            finish(&pending, Some(read(&LAST_INCREASE)));
        } else if frame - pending.edge_frame >= DAMAGE_WAIT_FRAMES {
            assign(&PENDING, None);
            finish(&pending, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_sweetspot_by_damage() {
        let hitboxes = [(0, 12.0), (1, 8.0), (2, 8.0)];
        assert_eq!(narrow(&hitboxes, 12.0), vec![(0, 12.0)]);
    }

    #[test]
    fn picks_sourspot_and_keeps_ties() {
        let hitboxes = [(0, 12.0), (1, 8.0), (2, 8.0)];
        assert_eq!(narrow(&hitboxes, 8.0), vec![(1, 8.0), (2, 8.0)]);
    }

    #[test]
    fn survives_scaled_damage() {
        // e.g. a stale move dealing 95% damage still matches the right hitbox
        let hitboxes = [(0, 12.0), (1, 8.0)];
        assert_eq!(narrow(&hitboxes, 11.4), vec![(0, 12.0)]);
        assert_eq!(narrow(&hitboxes, 7.6), vec![(1, 8.0)]);
    }

    #[test]
    fn formats_lists() {
        assert_eq!(format_ids(&[1]), "#1");
        assert_eq!(format_ids(&[0, 1, 2]), "#0/#1/#2");
        assert_eq!(format_ids(&[0, 1, 2, 3]), "#0/#1/#2+");
    }
}
