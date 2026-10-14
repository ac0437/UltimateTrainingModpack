//! "Combo Check": shows when a frame-tight combo was dropped.
//!
//! Two pieces of feedback, both driven by the CPU's hitstun:
//!
//! 1. Hitstun left: the moment *you* become actionable while the CPU is still in hitstun,
//!    show how many hitstun frames it has left (green, "+N"). Compare that to your next
//!    move's startup to know whether the follow-up can still connect.
//! 2. Combo dropped: the moment the CPU leaves hitstun without being hit again, show a red
//!    "Combo Dropped" notification and flash the CPU white. If you were not yet actionable
//!    when it got out, the number of frames you were late by is shown once you can act.
//!
//! Ending hitstun by dying, being grabbed, or being knocked down/teching is not a drop.

use skyline::nn::ui2d::ResColor;
use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use crate::common::consts::{FighterId, OnOff};
use crate::common::{is_in_hitstun, is_in_tech, try_get_module_accessor, MENU};
use crate::training::combo::is_actionable;
use crate::training::input_record::set_color_rgb_2;
use crate::training::ui::notifications::{clear_notification, color_notification};
use training_mod_sync::*;

const HITSTUN_LEFT_HEADER: &str = "Hitstun Left";
const DROPPED_HEADER: &str = "Combo Dropped";
const LATE_HEADER: &str = "Frames Late";

const NOTIFICATION_FRAMES: u32 = 90;
/// How long the CPU flashes white after a drop
const FLASH_FRAMES_ON_DROP: u32 = 12;
/// Stop waiting for the player to become actionable after this many frames
const LATE_TIMEOUT_FRAMES: u32 = 180;

const GREEN: ResColor = ResColor {
    r: 31,
    g: 198,
    b: 0,
    a: 255,
};
const RED: ResColor = ResColor {
    r: 200,
    g: 8,
    b: 8,
    a: 255,
};

static CPU_WAS_IN_HITSTUN: RwLock<bool> = RwLock::new(false);
static PLAYER_WAS_ACTIONABLE: RwLock<bool> = RwLock::new(false);
// A drop happened and we are waiting for the player to become actionable
static DROP_PENDING: RwLock<bool> = RwLock::new(false);
static FRAMES_SINCE_DROP: RwLock<u32> = RwLock::new(0);
static FLASH_FRAMES: RwLock<u32> = RwLock::new(0);

fn clear_notifications() {
    clear_notification(HITSTUN_LEFT_HEADER);
    clear_notification(DROPPED_HEADER);
    clear_notification(LATE_HEADER);
}

/// Hitstun frames the fighter has left, as tracked by the game.
unsafe fn hitstun_left(module_accessor: *mut BattleObjectModuleAccessor) -> i32 {
    WorkModule::get_float(
        module_accessor,
        *FIGHTER_INSTANCE_WORK_ID_FLOAT_DAMAGE_REACTION_FRAME,
    )
    .ceil() as i32
}

/// True if the fighter's current status means hitstun ended for a reason that is not an escape.
unsafe fn ended_without_escape(module_accessor: *mut BattleObjectModuleAccessor) -> bool {
    let status = StatusModule::status_kind(module_accessor);
    [
        *FIGHTER_STATUS_KIND_DEAD,
        *FIGHTER_STATUS_KIND_REBIRTH,
        *FIGHTER_STATUS_KIND_CAPTURE_PULLED,
        *FIGHTER_STATUS_KIND_CAPTURE_WAIT,
        *FIGHTER_STATUS_KIND_CAPTURE_DAMAGE,
        *FIGHTER_STATUS_KIND_THROWN,
        *FIGHTER_STATUS_KIND_DOWN,
        *FIGHTER_STATUS_KIND_DOWN_WAIT,
        *FIGHTER_STATUS_KIND_DOWN_WAIT_CONTINUE,
        *FIGHTER_STATUS_KIND_LAY_DOWN,
    ]
    .contains(&status)
        || is_in_tech(&mut *module_accessor)
}

/// Tracks the combo and shows notifications. Runs once per frame, from the player's update.
pub unsafe fn once_per_frame(module_accessor: &mut BattleObjectModuleAccessor) {
    if read(&MENU).combo_check != OnOff::ON {
        return;
    }
    let entry_id = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry_id != FighterId::Player as i32 {
        return;
    }
    let (Some(player), Some(cpu)) = (
        try_get_module_accessor(FighterId::Player),
        try_get_module_accessor(FighterId::CPU),
    ) else {
        return;
    };

    let cpu_in_hitstun = is_in_hitstun(&mut *cpu);
    let cpu_was_in_hitstun = read(&CPU_WAS_IN_HITSTUN);
    let player_actionable = is_actionable(player);
    let player_just_actionable = player_actionable && !read(&PLAYER_WAS_ACTIONABLE);

    if cpu_in_hitstun {
        // The CPU is (still or again) in hitstun, so there is no pending drop
        assign(&DROP_PENDING, false);
        if player_just_actionable {
            let left = hitstun_left(cpu);
            if left > 0 {
                clear_notifications();
                color_notification(
                    HITSTUN_LEFT_HEADER.to_string(),
                    format!("{left:+}"),
                    NOTIFICATION_FRAMES,
                    GREEN,
                );
            }
        }
    } else if cpu_was_in_hitstun && !ended_without_escape(cpu) {
        // The CPU just got out of hitstun
        clear_notifications();
        color_notification(
            DROPPED_HEADER.to_string(),
            "!".to_string(),
            NOTIFICATION_FRAMES,
            RED,
        );
        assign(&FLASH_FRAMES, FLASH_FRAMES_ON_DROP);
        // If the player could already act, they had their chance and missed it.
        // Otherwise wait and report how late they were.
        assign(&DROP_PENDING, !player_actionable);
        assign(&FRAMES_SINCE_DROP, 0);
    } else if read(&DROP_PENDING) {
        let frames = read(&FRAMES_SINCE_DROP) + 1;
        assign(&FRAMES_SINCE_DROP, frames);
        if player_just_actionable {
            clear_notifications();
            color_notification(
                LATE_HEADER.to_string(),
                format!("-{frames}"),
                NOTIFICATION_FRAMES,
                RED,
            );
            assign(&DROP_PENDING, false);
        } else if frames > LATE_TIMEOUT_FRAMES {
            assign(&DROP_PENDING, false);
        }
    }

    assign(&CPU_WAS_IN_HITSTUN, cpu_in_hitstun);
    assign(&PLAYER_WAS_ACTIONABLE, player_actionable);
}

/// Flashes the CPU white after a drop. Runs for every fighter, and acts on the CPU only,
/// so it happens in the CPU's own update and is not overwritten by State Tint.
pub unsafe fn apply_flash(module_accessor: &mut BattleObjectModuleAccessor) {
    if read(&MENU).combo_check != OnOff::ON {
        return;
    }
    let entry_id = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry_id != FighterId::CPU as i32 {
        return;
    }
    let frames = read(&FLASH_FRAMES);
    if frames == 0 {
        return;
    }
    assign(&FLASH_FRAMES, frames - 1);
    set_color_rgb_2(
        module_accessor,
        1.0,
        1.0,
        1.0,
        *MODEL_COLOR_TYPE_COLOR_BLEND,
    );
}
