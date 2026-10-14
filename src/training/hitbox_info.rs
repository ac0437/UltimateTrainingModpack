//! "Hitbox Info": shows which hitbox of yours connected.
//!
//! When one of the player's attacks connects, a notification shows the active hitbox ID(s).
//! The game reports "an attack connected", not which hitbox did it, so:
//!   - one active hitbox: exact. Shows `#ID` and its damage, in that ID's Hitbox Visualization color.
//!   - several active hitboxes (e.g. overlapping sweetspot/sourspot): shows the list of active IDs.
//!
//! The full data (damage, size, angle, knockback) of every active hitbox is written to the
//! mod's log file on each connect.
//!
//! This does not identify the opponent's hurtbox.

use skyline::nn::ui2d::ResColor;
use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use crate::common::consts::{FighterId, OnOff};
use crate::common::MENU;
use crate::hitbox_visualizer::ID_COLORS;
use crate::info;
use crate::training::ui::notifications::{clear_notification, color_notification};
use training_mod_sync::*;

const HEADER: &str = "Hitbox";
const NOTIFICATION_FRAMES: u32 = 90;
/// Hitbox IDs 0..8, the same range the Hitbox Visualization option covers
const HITBOX_IDS: i32 = 8;
/// Most IDs listed in the notification before it is cut off with a "+"
const MAX_IDS_SHOWN: usize = 3;

const WHITE: ResColor = ResColor {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

static WAS_INFLICTING: RwLock<bool> = RwLock::new(false);

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

pub unsafe fn once_per_frame(module_accessor: &mut BattleObjectModuleAccessor) {
    if read(&MENU).hitbox_info != OnOff::ON {
        return;
    }
    let entry_id = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry_id != FighterId::Player as i32 {
        return;
    }

    // React once when a hit starts, not on every frame it stays true
    let inflicting = AttackModule::is_infliction(module_accessor, *COLLISION_KIND_MASK_HIT);
    let was_inflicting = read(&WAS_INFLICTING);
    assign(&WAS_INFLICTING, inflicting);
    if !inflicting || was_inflicting {
        return;
    }

    let active: Vec<i32> = (0..HITBOX_IDS)
        .filter(|&id| AttackModule::is_attack(module_accessor, id, false))
        .collect();
    if active.is_empty() {
        return;
    }

    for &id in &active {
        let data = *AttackModule::attack_data(module_accessor, id, false);
        info!(
            "Hitbox #{id} active on hit: damage {:.1}, size {:.1}, angle {}, KBG {}, FKB {}, BKB {}",
            data.power, data.size, data.vector, data.r_eff, data.r_fix, data.r_add
        );
    }

    let (message, color) = if let [only] = active[..] {
        let data = *AttackModule::attack_data(module_accessor, only, false);
        (format!("#{only} {:.1}%", data.power), id_color(only))
    } else {
        (format_ids(&active), WHITE)
    };

    clear_notification(HEADER);
    color_notification(HEADER.to_string(), message, NOTIFICATION_FRAMES, color);
}
