//! "Hitstun Left": when you can act again while the CPU is still in hitstun, shows how many
//! hitstun frames the CPU has left (e.g. "5f"). Compare it to your next move's startup to
//! know whether the follow-up can still connect.
//!
//! The number is drawn through info_box, so it shares the on-screen box with the hitbox and
//! frame advantage instead of replacing them.

use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use crate::common::consts::{FighterId, OnOff};
use crate::common::{is_in_hitstun, try_get_module_accessor, MENU};
use crate::training::combo::is_actionable;
use crate::training::info_box;
use training_mod_sync::*;

static PLAYER_WAS_ACTIONABLE: RwLock<bool> = RwLock::new(false);

/// Hitstun frames the fighter has left, as tracked by the game.
unsafe fn hitstun_left(module_accessor: *mut BattleObjectModuleAccessor) -> i32 {
    WorkModule::get_float(
        module_accessor,
        *FIGHTER_INSTANCE_WORK_ID_FLOAT_DAMAGE_REACTION_FRAME,
    )
    .ceil() as i32
}

/// Runs once per frame, from the player's update.
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

    let player_actionable = is_actionable(player);
    let player_just_actionable = player_actionable && !read(&PLAYER_WAS_ACTIONABLE);
    assign(&PLAYER_WAS_ACTIONABLE, player_actionable);

    if player_just_actionable && is_in_hitstun(&mut *cpu) {
        let left = hitstun_left(cpu);
        if left > 0 {
            info_box::show_hitstun_left(left);
        }
    }
}
