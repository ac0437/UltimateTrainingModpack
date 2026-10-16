//! "Hitbox Info": shows which hitbox of yours connected.
//!
//! The game says "one of your attacks connected", not which hitbox did it, so this works it out:
//!
//!   1. Every hitbox active when the hit landed is recorded with its damage and its position in
//!      the world (a sphere, or a capsule between two points).
//!   2. The hitboxes whose damage matches what the opponent actually lost are kept.
//!   3. Of those, the ones that physically overlap the opponent's body (approximated as a
//!      vertical capsule) are kept, and the lowest-numbered one is reported.
//!
//! Display: `#1 F8` (hitbox 1, on frame 8 of the move) is a confident answer; `#1? F8` means no hitbox overlapped the
//! approximated body and the nearest one is shown as a guess. The opponent's real hurtboxes are
//! not read, so overlap is approximate. Nothing is written to the SD card, so it costs no
//! frame time beyond a few joint lookups when a hit lands.

use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::hash40;
use smash::lib::lua_const::*;
use smash::phx::{Hash40, Vector3f};

use crate::common::consts::{FighterId, OnOff};
use crate::common::{try_get_module_accessor, MENU};
use crate::training::info_box;
use training_mod_sync::*;

/// Hitbox IDs 0..8, the same range the Hitbox Visualization option covers
const HITBOX_IDS: i32 = 8;
/// Frames to wait after a hit for the opponent's damage to change before giving up
const DAMAGE_WAIT_FRAMES: u32 = 4;
/// Hitboxes whose damage is this close to the best match are kept as ties
const DAMAGE_TOLERANCE: f32 = 0.25;
/// Added to the animation frame to match the frame numbering of frame data. The animation
/// starts at 0; if the shown frame is one lower than your frame data, set this to 1.
const MOVE_FRAME_OFFSET: i32 = 0;
/// Opponent body radius as a fraction of their height (hip to feet, doubled)
const BODY_RADIUS_FRACTION: f32 = 0.25;

type P3 = [f32; 3];

/// An active hitbox in world space: a sphere, or a capsule from `a` to `b`
#[derive(Clone, Copy, Debug)]
struct Hitbox {
    id: i32,
    damage: f32,
    size: f32,
    a: P3,
    b: P3,
}

/// The opponent's body approximated as a vertical capsule
#[derive(Clone, Copy, Debug)]
struct Body {
    bottom: P3,
    top: P3,
    radius: f32,
}

/// A hit waiting for the opponent's damage to change
#[derive(Clone)]
struct Pending {
    hitboxes: Vec<Hitbox>,
    body: Option<Body>,
    edge_frame: u32,
    /// The attacker's animation frame when the hit landed
    move_frame: i32,
}

static FRAME: RwLock<u32> = RwLock::new(0);
static WAS_INFLICTING: RwLock<bool> = RwLock::new(false);
// The opponent's damage last frame, or negative before the first reading
static LAST_CPU_DAMAGE: RwLock<f32> = RwLock::new(-1.0);
// The most recent damage increase and the frame it happened on
static LAST_INCREASE: RwLock<f32> = RwLock::new(0.0);
static LAST_INCREASE_FRAME: RwLock<Option<u32>> = RwLock::new(None);
static PENDING: RwLock<Option<Pending>> = RwLock::new(None);

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: P3, b: P3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Shortest distance between segments p1-q1 and p2-q2 (a point is a zero-length segment)
fn segment_distance(p1: P3, q1: P3, p2: P3, q2: P3) -> f32 {
    let d1 = sub(q1, p1);
    let d2 = sub(q2, p2);
    let r = sub(p1, p2);
    let a = dot(d1, d1);
    let e = dot(d2, d2);
    let f = dot(d2, r);
    let eps = 1e-8;
    let (s, t);
    if a <= eps && e <= eps {
        s = 0.0;
        t = 0.0;
    } else if a <= eps {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = dot(d1, r);
        if e <= eps {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > eps {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    let c1 = [p1[0] + d1[0] * s, p1[1] + d1[1] * s, p1[2] + d1[2] * s];
    let c2 = [p2[0] + d2[0] * t, p2[1] + d2[1] * t, p2[2] + d2[2] * t];
    let d = sub(c1, c2);
    dot(d, d).sqrt()
}

/// Distance between the surfaces of a hitbox and the body; negative when they overlap
fn gap(hitbox: &Hitbox, body: &Body) -> f32 {
    segment_distance(hitbox.a, hitbox.b, body.bottom, body.top) - hitbox.size - body.radius
}

/// The hitboxes whose damage best matches the damage dealt, plus any tied with the best
fn narrow(hitboxes: &[Hitbox], dealt: f32) -> Vec<Hitbox> {
    let best = hitboxes
        .iter()
        .map(|h| (h.damage - dealt).abs())
        .fold(f32::MAX, f32::min);
    hitboxes
        .iter()
        .copied()
        .filter(|h| (h.damage - dealt).abs() <= best + DAMAGE_TOLERANCE)
        .collect()
}

/// Picks the hitbox that connected. Returns it and whether the answer is confident.
fn choose(candidates: &[Hitbox], body: Option<&Body>) -> Option<(Hitbox, bool)> {
    if let [only] = candidates {
        return Some((*only, true));
    }
    let body = body?;
    let overlapping = candidates
        .iter()
        .filter(|h| gap(h, body) <= 0.0)
        .min_by_key(|h| h.id);
    if let Some(h) = overlapping {
        return Some((*h, true));
    }
    candidates
        .iter()
        .min_by(|a, b| gap(a, body).total_cmp(&gap(b, body)))
        .map(|h| (*h, false))
}

unsafe fn joint_position(
    module_accessor: *mut BattleObjectModuleAccessor,
    node: u64,
    offset: P3,
) -> P3 {
    let offset = Vector3f {
        x: offset[0],
        y: offset[1],
        z: offset[2],
    };
    let mut out = Vector3f {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    ModelModule::joint_global_position_with_offset(
        module_accessor,
        Hash40 { hash: node },
        &offset,
        &mut out,
        true,
    );
    [out.x, out.y, out.z]
}

/// Every active hitbox of the player, in world space
unsafe fn active_hitboxes(module_accessor: *mut BattleObjectModuleAccessor) -> Vec<Hitbox> {
    (0..HITBOX_IDS)
        .filter_map(|id| {
            if !AttackModule::is_attack(module_accessor, id, false) {
                return None;
            }
            let data = *AttackModule::attack_data(module_accessor, id, false);
            let a = joint_position(module_accessor, data.node, [data.x, data.y, data.z]);
            let is_capsule = data.x2 != 0.0 || data.y2 != 0.0 || data.z2 != 0.0;
            let b = if is_capsule {
                joint_position(module_accessor, data.node, [data.x2, data.y2, data.z2])
            } else {
                a
            };
            Some(Hitbox {
                id,
                damage: data.power,
                size: data.size,
                a,
                b,
            })
        })
        .collect()
}

/// The opponent's body, from their feet and hip
unsafe fn opponent_body(cpu: *mut BattleObjectModuleAccessor) -> Option<Body> {
    let mut hip = Vector3f {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    ModelModule::joint_global_position(
        cpu,
        Hash40 {
            hash: hash40("hip"),
        },
        &mut hip,
        true,
    );
    let feet_y = PostureModule::pos_y(cpu);
    let height = (hip.y - feet_y) * 2.0;
    if height <= 0.0 {
        return None;
    }
    Some(Body {
        bottom: [hip.x, feet_y, hip.z],
        top: [hip.x, feet_y + height, hip.z],
        radius: height * BODY_RADIUS_FRACTION,
    })
}

fn finish(pending: &Pending, dealt: Option<f32>) {
    let candidates = match dealt {
        Some(dealt) => narrow(&pending.hitboxes, dealt),
        None => pending.hitboxes.clone(),
    };
    let chosen = choose(&candidates, pending.body.as_ref());

    let message = match chosen {
        Some((hitbox, confident)) => {
            let mark = if confident { "" } else { "?" };
            format!("#{}{mark} F{}", hitbox.id, pending.move_frame)
        }
        None => format!("F{}", pending.move_frame),
    };

    info_box::show_hitbox(message);
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
    let cpu = try_get_module_accessor(FighterId::CPU);

    // Track the opponent's damage so a hit can be matched to the damage it did
    if let Some(cpu) = cpu {
        let damage = DamageModule::damage(cpu, 0);
        let previous = read(&LAST_CPU_DAMAGE);
        if previous >= 0.0 && damage > previous + 0.001 {
            assign(&LAST_INCREASE, damage - previous);
            assign(&LAST_INCREASE_FRAME, Some(frame));
        }
        assign(&LAST_CPU_DAMAGE, damage);
    }

    // A new hit: remember which hitboxes were active and where everyone was
    let inflicting = AttackModule::is_infliction(module_accessor, *COLLISION_KIND_MASK_HIT);
    let was_inflicting = read(&WAS_INFLICTING);
    assign(&WAS_INFLICTING, inflicting);
    if inflicting && !was_inflicting {
        let hitboxes = active_hitboxes(module_accessor);
        if !hitboxes.is_empty() {
            assign(
                &PENDING,
                Some(Pending {
                    hitboxes,
                    body: cpu.and_then(|cpu| opponent_body(cpu)),
                    edge_frame: frame,
                    move_frame: MotionModule::frame(module_accessor).floor() as i32
                        + MOVE_FRAME_OFFSET,
                }),
            );
        }
    }

    // Resolve a pending hit once the damage shows up, or give up after a few frames.
    // Damage can land a frame before the game reports the hit, hence the `+ 1`.
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

    fn hb(id: i32, damage: f32, at: P3) -> Hitbox {
        Hitbox {
            id,
            damage,
            size: 3.0,
            a: at,
            b: at,
        }
    }

    const BODY: Body = Body {
        bottom: [0.0, 0.0, 0.0],
        top: [0.0, 12.0, 0.0],
        radius: 3.0,
    };

    #[test]
    fn damage_picks_sweetspot() {
        let hs = [hb(0, 12.0, [0.0; 3]), hb(1, 8.0, [0.0; 3])];
        let n = narrow(&hs, 12.0);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].id, 0);
    }

    #[test]
    fn geometry_breaks_equal_damage_ties() {
        // all 4.8; only #2 reaches the body
        let hs = [
            hb(0, 4.8, [30.0, 6.0, 0.0]),
            hb(1, 4.8, [20.0, 6.0, 0.0]),
            hb(2, 4.8, [5.0, 6.0, 0.0]),
        ];
        let (picked, confident) = choose(&narrow(&hs, 4.8), Some(&BODY)).unwrap();
        assert_eq!(picked.id, 2);
        assert!(confident);
    }

    #[test]
    fn lowest_id_wins_when_several_overlap() {
        let hs = [hb(1, 4.8, [5.0, 6.0, 0.0]), hb(2, 4.8, [4.0, 6.0, 0.0])];
        assert_eq!(choose(&hs, Some(&BODY)).unwrap().0.id, 1);
    }

    #[test]
    fn nearest_is_a_guess() {
        let hs = [hb(0, 4.8, [30.0, 6.0, 0.0]), hb(1, 4.8, [20.0, 6.0, 0.0])];
        let (picked, confident) = choose(&hs, Some(&BODY)).unwrap();
        assert_eq!(picked.id, 1);
        assert!(!confident);
    }

    #[test]
    fn capsule_distance() {
        let d = segment_distance([0.0; 3], [10.0, 0.0, 0.0], [5.0, 4.0, 0.0], [5.0, 8.0, 0.0]);
        assert!((d - 4.0).abs() < 1e-4);
    }
}
