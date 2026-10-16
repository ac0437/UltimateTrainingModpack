//! Combo practice helpers, both built on noticing when the CPU leaves hitstun:
//!
//!   - "Combo Over Flash": flashes the CPU white when it escapes hitstun without being hit
//!     again, so you can see the moment a combo ends.
//!   - "Combo Auto-Reset": a moment after a combo ends, reloads the save state and keeps score
//!     (completed combos / attempts, and the current streak).
//!
//! Ending hitstun by dying, being grabbed, or being knocked down/teching counts as a completed
//! combo and not as a drop: those end the combo for other reasons than the CPU getting out.

use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use crate::common::consts::{FighterId, OnOff};
use crate::common::{is_in_hitstun, is_in_tech, try_get_module_accessor, MENU};
use crate::training::input_record::set_color_rgb_2;
use crate::training::{info_box, save_states};
use training_mod_sync::*;

/// How long the CPU flashes white
const FLASH_FRAMES_ON_DROP: u32 = 12;
/// Frames between a combo ending and the save state reloading, long enough to read the result
const RESET_DELAY_FRAMES: u32 = 45;
/// Frames after a drop spent looking for what the CPU did to get out
const ESCAPE_WATCH_FRAMES: u32 = 12;

static CPU_WAS_IN_HITSTUN: RwLock<bool> = RwLock::new(false);
static FLASH_FRAMES: RwLock<u32> = RwLock::new(0);
static ATTEMPTS: RwLock<u32> = RwLock::new(0);
static SUCCESSES: RwLock<u32> = RwLock::new(0);
static STREAK: RwLock<u32> = RwLock::new(0);
static RESET_IN: RwLock<u32> = RwLock::new(0);
static ESCAPE_WATCH: RwLock<u32> = RwLock::new(0);

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

/// What the CPU did to get out, from its status, if it is doing something recognizable
unsafe fn escape_reason(module_accessor: *mut BattleObjectModuleAccessor) -> Option<&'static str> {
    let status = StatusModule::status_kind(module_accessor);
    let is = |kinds: &[i32]| kinds.contains(&status);
    if is(&[*FIGHTER_STATUS_KIND_ESCAPE_AIR]) {
        Some("air dodge")
    } else if is(&[*FIGHTER_STATUS_KIND_ESCAPE]) {
        Some("spot dodge")
    } else if is(&[*FIGHTER_STATUS_KIND_ESCAPE_F, *FIGHTER_STATUS_KIND_ESCAPE_B]) {
        Some("roll")
    } else if is(&[
        *FIGHTER_STATUS_KIND_GUARD_ON,
        *FIGHTER_STATUS_KIND_GUARD,
        *FIGHTER_STATUS_KIND_GUARD_DAMAGE,
    ]) {
        Some("shield")
    } else if is(&[
        *FIGHTER_STATUS_KIND_JUMP_SQUAT,
        *FIGHTER_STATUS_KIND_JUMP,
        *FIGHTER_STATUS_KIND_JUMP_AERIAL,
    ]) {
        Some("jump")
    } else if is(&[*FIGHTER_STATUS_KIND_ATTACK_AIR]) {
        Some("aerial")
    } else if is(&[
        *FIGHTER_STATUS_KIND_ATTACK,
        *FIGHTER_STATUS_KIND_ATTACK_S3,
        *FIGHTER_STATUS_KIND_ATTACK_HI3,
        *FIGHTER_STATUS_KIND_ATTACK_LW3,
        *FIGHTER_STATUS_KIND_ATTACK_S4,
        *FIGHTER_STATUS_KIND_ATTACK_HI4,
        *FIGHTER_STATUS_KIND_ATTACK_LW4,
        *FIGHTER_STATUS_KIND_ATTACK_DASH,
    ]) {
        Some("attack")
    } else if is(&[
        *FIGHTER_STATUS_KIND_SPECIAL_N,
        *FIGHTER_STATUS_KIND_SPECIAL_S,
        *FIGHTER_STATUS_KIND_SPECIAL_HI,
        *FIGHTER_STATUS_KIND_SPECIAL_LW,
    ]) {
        Some("special")
    } else {
        None
    }
}

/// The score as `successes/attempts`
fn score_text() -> String {
    format!("{}/{}", read(&SUCCESSES), read(&ATTEMPTS))
}

fn show_drop(reason: Option<&str>) {
    let reason = reason.map(|r| format!(": {r}")).unwrap_or_default();
    info_box::show_result(format!("Drop{reason} {}", score_text()), false);
}

fn show_completed() {
    let streak = read(&STREAK);
    let streak = if streak >= 2 {
        format!(" x{streak}")
    } else {
        String::new()
    };
    info_box::show_result(format!("Combo{streak} {}", score_text()), true);
}

/// Watches the CPU's hitstun. Runs once per frame, from the player's update.
pub unsafe fn once_per_frame(module_accessor: &mut BattleObjectModuleAccessor) {
    let flash = read(&MENU).combo_check == OnOff::ON;
    let auto_reset = read(&MENU).auto_reset == OnOff::ON;
    let entry_id = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry_id != FighterId::Player as i32 {
        return;
    }
    if !auto_reset {
        // Turning the option off and on again starts the score over
        assign(&ATTEMPTS, 0);
        assign(&SUCCESSES, 0);
        assign(&STREAK, 0);
        assign(&RESET_IN, 0);
        assign(&ESCAPE_WATCH, 0);
    }
    if !flash && !auto_reset {
        return;
    }
    let Some(cpu) = try_get_module_accessor(FighterId::CPU) else {
        return;
    };

    // A save state load kills and respawns the fighters, which is not a combo ending
    if save_states::is_loading() {
        assign(&CPU_WAS_IN_HITSTUN, false);
        assign(&RESET_IN, 0);
        assign(&ESCAPE_WATCH, 0);
        return;
    }

    let cpu_in_hitstun = is_in_hitstun(&mut *cpu);
    if read(&CPU_WAS_IN_HITSTUN) && !cpu_in_hitstun {
        if ended_without_escape(cpu) {
            if auto_reset {
                assign(&ATTEMPTS, read(&ATTEMPTS) + 1);
                assign(&SUCCESSES, read(&SUCCESSES) + 1);
                assign(&STREAK, read(&STREAK) + 1);
                show_completed();
                assign(&RESET_IN, RESET_DELAY_FRAMES);
            }
        } else {
            if flash {
                assign(&FLASH_FRAMES, FLASH_FRAMES_ON_DROP);
            }
            if auto_reset {
                assign(&ATTEMPTS, read(&ATTEMPTS) + 1);
                assign(&STREAK, 0);
                show_drop(None);
                assign(&ESCAPE_WATCH, ESCAPE_WATCH_FRAMES);
                assign(&RESET_IN, RESET_DELAY_FRAMES);
            }
        }
    }
    assign(&CPU_WAS_IN_HITSTUN, cpu_in_hitstun);

    // Fill in what the CPU did once it does something
    let watch = read(&ESCAPE_WATCH);
    if watch > 0 {
        if let Some(reason) = escape_reason(cpu) {
            show_drop(Some(reason));
            assign(&ESCAPE_WATCH, 0);
        } else {
            assign(&ESCAPE_WATCH, watch - 1);
        }
    }

    // Reload the save state a moment after the combo ended
    let reset_in = read(&RESET_IN);
    if reset_in > 0 {
        assign(&RESET_IN, reset_in - 1);
        if reset_in == 1 {
            save_states::request_reset();
        }
    }
}

/// Flashes the CPU white after the combo ends. Runs for every fighter, and acts on the CPU
/// only, so it happens in the CPU's own update and is not overwritten by State Tint.
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
