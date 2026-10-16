//! One shared on-screen box for the combo information the mod shows.
//!
//! The training layout has a single notification slot, and it shows queued notifications one
//! at a time, so a new message used to push out the one before it. This module keeps the latest
//! pieces of information and draws them together, as two lines:
//!
//! - top line: the hit, as `#1 F8`, with the frame advantage added once it is known
//!   (`#1 F8 +3`)
//! - bottom line: how the combo ended, as `Combo x4 8/12` or `Drop: air dodge 7/12`
//!
//! Pieces that are not available are left out. A new hit clears the frame advantage and the
//! result of the previous one, since they belong to the previous hit.

use skyline::nn::ui2d::ResColor;
use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use crate::common::consts::FighterId;
use crate::training::ui::notifications::{clear_all_notifications, color_notification};
use training_mod_sync::*;

/// How long the box stays up after the last update
const DISPLAY_FRAMES: u32 = 240;

const WHITE: ResColor = ResColor {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};
const RED: ResColor = ResColor {
    r: 200,
    g: 8,
    b: 8,
    a: 255,
};
const GREEN: ResColor = ResColor {
    r: 31,
    g: 198,
    b: 0,
    a: 255,
};

/// How a combo ended
#[derive(Clone)]
struct ResultLine {
    text: String,
    /// True for a completed combo, false for a drop
    good: bool,
}

static HITBOX: RwLock<Option<String>> = RwLock::new(None);
static FRAME_ADVANTAGE: RwLock<Option<i32>> = RwLock::new(None);
static RESULT: RwLock<Option<ResultLine>> = RwLock::new(None);
static FRAMES_LEFT: RwLock<u32> = RwLock::new(0);

/// Rebuilds the on-screen notification from whatever is currently set
fn refresh() {
    let hitbox = read_clone(&HITBOX);
    let frame_advantage = read(&FRAME_ADVANTAGE);
    let result = read_clone(&RESULT);

    let mut top_parts: Vec<String> = Vec::new();
    if let Some(hitbox) = &hitbox {
        top_parts.push(hitbox.clone());
    }
    if let Some(frames) = frame_advantage {
        top_parts.push(format!("{frames:+}"));
    }
    let top = top_parts.join(" ");

    let color = match (&result, frame_advantage) {
        (Some(result), _) if result.good => GREEN,
        (Some(_), _) => RED,
        (None, Some(frames)) if frames < 0 => RED,
        (None, Some(frames)) if frames > 0 => GREEN,
        _ => WHITE,
    };

    let (header, message) = match (top.is_empty(), result) {
        (true, None) => {
            clear_all_notifications();
            return;
        }
        (false, Some(result)) => (top, result.text),
        (true, Some(result)) => ("Combo".to_string(), result.text),
        (false, None) => (
            if hitbox.is_some() {
                "Hitbox".to_string()
            } else {
                "Frame Advantage".to_string()
            },
            top,
        ),
    };

    let frames = read(&FRAMES_LEFT);
    clear_all_notifications();
    color_notification(header, message, frames + 1, color);
}

fn touch() {
    assign(&FRAMES_LEFT, DISPLAY_FRAMES);
}

/// A hitbox of the player's connected, e.g. `#1 F8`. This starts a new hit, so the frame
/// advantage and result of the previous one are cleared.
pub fn show_hitbox(text: String) {
    assign(&FRAME_ADVANTAGE, None);
    assign(&RESULT, None);
    assign(&HITBOX, Some(text));
    touch();
    refresh();
}

pub fn show_frame_advantage(frames: i32) {
    assign(&FRAME_ADVANTAGE, Some(frames));
    touch();
    refresh();
}

/// How the combo ended. Replaces any earlier result for the same hit.
pub fn show_result(text: String, good: bool) {
    assign(&RESULT, Some(ResultLine { text, good }));
    touch();
    refresh();
}

/// Counts down what is on screen. Runs every frame, from the player's update.
pub unsafe fn tick(module_accessor: &mut BattleObjectModuleAccessor) {
    let entry_id = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry_id != FighterId::Player as i32 {
        return;
    }

    let frames = read(&FRAMES_LEFT);
    if frames == 0 {
        return;
    }
    assign(&FRAMES_LEFT, frames - 1);
    if frames == 1 {
        assign(&HITBOX, None);
        assign(&FRAME_ADVANTAGE, None);
        assign(&RESULT, None);
        refresh();
    }
}
