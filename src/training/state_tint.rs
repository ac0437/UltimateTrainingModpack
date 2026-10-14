//! "State Tint": colors each fighter's model to show what state it is in.
//!
//! Priority, highest first:
//!   1. Ledge option (hanging, getup, roll, jump, attack)
//!   2. Tech option (tech in place, roll forwards/backwards, missed tech and its getups)
//!   3. In hitstun: DI direction color, or red if no DI. Everything else: green
//!
//! Every color lives in the palette block below, so retuning is a one-line change.
//! The tint is applied with the same `ModelModule::set_color_rgb` blend that input
//! recording uses for its Standby/Recording/Lockout colors, and is skipped while that
//! system owns the CPU's color.

use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::hash40;
use smash::lib::lua_const::*;

use crate::common::consts::OnOff;
use crate::common::{is_fighter, is_in_hitstun, MENU};
use crate::training::input_record::{set_color_rgb_2, PossessionState, POSSESSION};
use training_mod_sync::*;

#[derive(Clone, Copy)]
struct Tint {
    r: f32,
    g: f32,
    b: f32,
}

const fn rgb(r: f32, g: f32, b: f32) -> Tint {
    Tint { r, g, b }
}

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

// Hitstun
const FREE: Tint = rgb(0.0, 1.0, 0.0); // green: not in hitstun
const HITSTUN: Tint = rgb(1.0, 0.0, 0.0); // red: in hitstun with no DI (stick near neutral)

// DI direction while in hitstun (absolute screen directions, not relative to the opponent).
// Green and red are left out on purpose: they already mean "free" and "hitstun, no DI".
const DI_RIGHT: Tint = rgb(0.2, 0.4, 1.0); // blue
const DI_UP_RIGHT: Tint = rgb(0.0, 1.0, 1.0); // cyan
const DI_UP: Tint = rgb(1.0, 1.0, 1.0); // white
const DI_UP_LEFT: Tint = rgb(1.0, 1.0, 0.0); // yellow
const DI_LEFT: Tint = rgb(1.0, 0.5, 0.0); // orange
const DI_DOWN_LEFT: Tint = rgb(1.0, 0.4, 0.8); // pink
const DI_DOWN: Tint = rgb(1.0, 0.0, 1.0); // magenta
const DI_DOWN_RIGHT: Tint = rgb(0.6, 0.2, 1.0); // purple

// Stick magnitude below this counts as "no DI"
const DI_DEADZONE: f32 = 0.3;

// Tech options
const TECH_IN_PLACE: Tint = rgb(0.0, 1.0, 1.0); // cyan (also wall / ceiling tech)
const TECH_ROLL_FORWARD: Tint = rgb(0.2, 0.4, 1.0); // blue
const TECH_ROLL_BACKWARD: Tint = rgb(0.6, 0.2, 1.0); // purple
const MISSED_TECH: Tint = rgb(1.0, 0.5, 0.0); // orange: lying on the ground
const MISSED_TECH_GETUP: Tint = rgb(1.0, 1.0, 0.0); // yellow: neutral getup
const MISSED_TECH_ATTACK: Tint = rgb(1.0, 0.0, 1.0); // magenta: getup attack
const MISSED_TECH_ROLL_FORWARD: Tint = TECH_ROLL_FORWARD;
const MISSED_TECH_ROLL_BACKWARD: Tint = TECH_ROLL_BACKWARD;

// Ledge options
const LEDGE_HANG: Tint = rgb(1.0, 0.9, 0.2); // gold: hanging, no option chosen yet
const LEDGE_NEUTRAL: Tint = rgb(1.0, 1.0, 1.0); // white
const LEDGE_ROLL: Tint = rgb(0.2, 0.4, 1.0); // blue
const LEDGE_JUMP: Tint = rgb(1.0, 0.4, 0.8); // pink
const LEDGE_ATTACK: Tint = rgb(0.6, 0.2, 1.0); // purple

// ---------------------------------------------------------------------------
// State -> color
// ---------------------------------------------------------------------------

unsafe fn ledge_tint(status: i32) -> Option<Tint> {
    Some(match status {
        s if s == *FIGHTER_STATUS_KIND_CLIFF_CATCH || s == *FIGHTER_STATUS_KIND_CLIFF_WAIT => {
            LEDGE_HANG
        }
        s if s == *FIGHTER_STATUS_KIND_CLIFF_CLIMB => LEDGE_NEUTRAL,
        s if s == *FIGHTER_STATUS_KIND_CLIFF_ESCAPE => LEDGE_ROLL,
        s if s == *FIGHTER_STATUS_KIND_CLIFF_JUMP1
            || s == *FIGHTER_STATUS_KIND_CLIFF_JUMP2
            || s == *FIGHTER_STATUS_KIND_CLIFF_JUMP3 =>
        {
            LEDGE_JUMP
        }
        s if s == *FIGHTER_STATUS_KIND_CLIFF_ATTACK => LEDGE_ATTACK,
        _ => return None,
    })
}

unsafe fn tech_tint(module_accessor: *mut BattleObjectModuleAccessor, status: i32) -> Option<Tint> {
    // Roll direction is read from the motion, the same way tech.rs swaps it in change_motion
    let motion = MotionModule::motion_kind(module_accessor);
    let rolling_forward = [
        hash40("passive_stand_f"),
        hash40("down_forward_u"),
        hash40("down_forward_d"),
    ]
    .contains(&motion);

    Some(match status {
        // Successful tech
        s if s == *FIGHTER_STATUS_KIND_PASSIVE
            || s == *FIGHTER_STATUS_KIND_PASSIVE_WALL
            || s == *FIGHTER_STATUS_KIND_PASSIVE_WALL_JUMP
            || s == *FIGHTER_STATUS_KIND_PASSIVE_CEIL =>
        {
            TECH_IN_PLACE
        }
        s if s == *FIGHTER_STATUS_KIND_PASSIVE_FB => {
            if rolling_forward {
                TECH_ROLL_FORWARD
            } else {
                TECH_ROLL_BACKWARD
            }
        }
        // Missed tech: lying on the ground
        s if s == *FIGHTER_STATUS_KIND_DOWN
            || s == *FIGHTER_STATUS_KIND_DOWN_WAIT
            || s == *FIGHTER_STATUS_KIND_DOWN_WAIT_CONTINUE
            || s == *FIGHTER_STATUS_KIND_LAY_DOWN
            || s == *FIGHTER_STATUS_KIND_SLIP_WAIT =>
        {
            MISSED_TECH
        }
        // Missed tech: getup options
        s if s == *FIGHTER_STATUS_KIND_DOWN_STAND || s == *FIGHTER_STATUS_KIND_SLIP_STAND => {
            MISSED_TECH_GETUP
        }
        s if s == *FIGHTER_STATUS_KIND_DOWN_STAND_ATTACK
            || s == *FIGHTER_STATUS_KIND_SLIP_STAND_ATTACK =>
        {
            MISSED_TECH_ATTACK
        }
        s if s == *FIGHTER_STATUS_KIND_DOWN_STAND_FB => {
            if rolling_forward {
                MISSED_TECH_ROLL_FORWARD
            } else {
                MISSED_TECH_ROLL_BACKWARD
            }
        }
        s if s == *FIGHTER_STATUS_KIND_SLIP_STAND_F => MISSED_TECH_ROLL_FORWARD,
        s if s == *FIGHTER_STATUS_KIND_SLIP_STAND_B => MISSED_TECH_ROLL_BACKWARD,
        _ => return None,
    })
}

/// Color for the DI the fighter took on this hit, or `None` if the stick was near neutral.
///
/// Reads the same work floats that directional_influence.rs writes for the CPU's DI option.
/// For a human-controlled fighter the game fills them from the stick when the hit lands.
unsafe fn di_tint(module_accessor: *mut BattleObjectModuleAccessor) -> Option<Tint> {
    let x = WorkModule::get_float(
        module_accessor,
        *FIGHTER_STATUS_DAMAGE_WORK_FLOAT_VECOR_CORRECT_STICK_X,
    );
    let y = WorkModule::get_float(
        module_accessor,
        *FIGHTER_STATUS_DAMAGE_WORK_FLOAT_VECOR_CORRECT_STICK_Y,
    );
    if (x * x + y * y).sqrt() < DI_DEADZONE {
        return None;
    }

    // Split the circle into eight 45-degree sectors centered on the compass directions
    let degrees = y.atan2(x).to_degrees();
    let sector = ((degrees + 22.5) / 45.0).floor().rem_euclid(8.0) as i32;
    Some(match sector {
        0 => DI_RIGHT,
        1 => DI_UP_RIGHT,
        2 => DI_UP,
        3 => DI_UP_LEFT,
        4 => DI_LEFT,
        5 => DI_DOWN_LEFT,
        6 => DI_DOWN,
        _ => DI_DOWN_RIGHT,
    })
}

unsafe fn pick_tint(module_accessor: *mut BattleObjectModuleAccessor) -> Tint {
    let status = StatusModule::status_kind(module_accessor);

    if let Some(tint) = ledge_tint(status) {
        return tint;
    }
    if let Some(tint) = tech_tint(module_accessor, status) {
        return tint;
    }
    if is_in_hitstun(&mut *module_accessor) {
        di_tint(module_accessor).unwrap_or(HITSTUN)
    } else {
        FREE
    }
}

// ---------------------------------------------------------------------------
// Per-frame hook
// ---------------------------------------------------------------------------

pub unsafe fn once_per_frame(module_accessor: &mut BattleObjectModuleAccessor) {
    if read(&MENU).state_tint != OnOff::ON || !is_fighter(module_accessor) {
        return;
    }

    // Input recording tints the CPU while it is being recorded or held in standby.
    // Don't fight it for the color.
    if read(&POSSESSION) != PossessionState::Player
        && WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID) == 1
    {
        return;
    }

    let tint = pick_tint(module_accessor);
    set_color_rgb_2(
        module_accessor,
        tint.r,
        tint.g,
        tint.b,
        *MODEL_COLOR_TYPE_COLOR_BLEND,
    );
}
