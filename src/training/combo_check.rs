//! "Combo Over Flash": flashes the CPU white when it escapes hitstun without being hit again,
//! so you can see the moment a combo ends.
//!
//! Ending hitstun by dying, being grabbed, or being knocked down/teching does not count: those
//! end the combo for other reasons than the CPU getting out of it.

use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use crate::common::consts::{FighterId, OnOff};
use crate::common::{is_in_hitstun, is_in_tech, try_get_module_accessor, MENU};
use crate::training::input_record::set_color_rgb_2;
use training_mod_sync::*;

/// How long the CPU flashes white
const FLASH_FRAMES_ON_DROP: u32 = 12;

static CPU_WAS_IN_HITSTUN: RwLock<bool> = RwLock::new(false);
static FLASH_FRAMES: RwLock<u32> = RwLock::new(0);

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

/// Watches the CPU's hitstun. Runs once per frame, from the player's update.
pub unsafe fn once_per_frame(module_accessor: &mut BattleObjectModuleAccessor) {
    if read(&MENU).combo_check != OnOff::ON {
        return;
    }
    let entry_id = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry_id != FighterId::Player as i32 {
        return;
    }
    let Some(cpu) = try_get_module_accessor(FighterId::CPU) else {
        return;
    };

    let cpu_in_hitstun = is_in_hitstun(&mut *cpu);
    if read(&CPU_WAS_IN_HITSTUN) && !cpu_in_hitstun && !ended_without_escape(cpu) {
        assign(&FLASH_FRAMES, FLASH_FRAMES_ON_DROP);
    }
    assign(&CPU_WAS_IN_HITSTUN, cpu_in_hitstun);
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
