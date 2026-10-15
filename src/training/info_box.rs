//! One shared on-screen box for the combo information the mod shows.
//!
//! The training layout has a single notification slot, and it shows queued notifications one
//! at a time, so a new message used to push out the one before it. This module keeps the latest
//! hitbox and the latest number (frame advantage or hitstun left) and draws them together:
//!
//!   - hitbox only:  header "Hitbox",        message "#2 12.0%"
//!   - number only:  header "Frame Advantage" / "Hitstun Left", message "+4" / "5f"
//!   - both:         header "#2 12.0%",       message "+4" (the number, in its own color)
//!
//! A new hit clears the previous number, since it belongs to the previous hit.

use skyline::nn::ui2d::ResColor;
use smash::app::lua_bind::*;
use smash::app::BattleObjectModuleAccessor;
use smash::lib::lua_const::*;

use crate::common::consts::FighterId;
use crate::training::ui::notifications::{clear_all_notifications, color_notification};
use training_mod_sync::*;

/// How long a hitbox stays on screen. Long enough to still be there when frame advantage
/// appears at the end of hitstun.
const HITBOX_FRAMES: u32 = 150;
const NUMBER_FRAMES: u32 = 90;

const RED: ResColor = ResColor {
    r: 200,
    g: 8,
    b: 8,
    a: 255,
};
const BLACK: ResColor = ResColor {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};
const GREEN: ResColor = ResColor {
    r: 31,
    g: 198,
    b: 0,
    a: 255,
};

#[derive(Clone)]
struct HitboxLine {
    text: String,
    color: ResColor,
    frames_left: u32,
}

#[derive(Clone, Copy)]
enum Number {
    FrameAdvantage(i32),
    HitstunLeft(i32),
}

#[derive(Clone, Copy)]
struct NumberLine {
    number: Number,
    frames_left: u32,
}

static HITBOX: RwLock<Option<HitboxLine>> = RwLock::new(None);
static NUMBER: RwLock<Option<NumberLine>> = RwLock::new(None);

/// (header label, message, color) for a number shown on its own
fn describe(number: Number) -> (&'static str, String, ResColor) {
    match number {
        Number::FrameAdvantage(frames) => (
            "Frame Advantage",
            format!("{frames:+}"),
            match frames {
                x if x < 0 => RED,
                0 => BLACK,
                _ => GREEN,
            },
        ),
        Number::HitstunLeft(frames) => ("Hitstun Left", format!("{frames}f"), GREEN),
    }
}

/// Rebuilds the on-screen notification from whatever is currently active
fn refresh() {
    let hitbox = read_clone(&HITBOX);
    let number = read(&NUMBER);

    let (header, message, color, frames) = match (hitbox, number) {
        (None, None) => {
            clear_all_notifications();
            return;
        }
        (Some(hitbox), None) => (
            "Hitbox".to_string(),
            hitbox.text,
            hitbox.color,
            hitbox.frames_left,
        ),
        (None, Some(line)) => {
            let (label, text, color) = describe(line.number);
            (label.to_string(), text, color, line.frames_left)
        }
        (Some(hitbox), Some(line)) => {
            let (_, text, color) = describe(line.number);
            (
                hitbox.text,
                text,
                color,
                hitbox.frames_left.max(line.frames_left),
            )
        }
    };

    clear_all_notifications();
    color_notification(header, message, frames + 1, color);
}

/// A hitbox of the player's connected. This starts a new hit, so any earlier number is cleared.
pub fn show_hitbox(text: String, color: ResColor) {
    assign(&NUMBER, None);
    assign(
        &HITBOX,
        Some(HitboxLine {
            text,
            color,
            frames_left: HITBOX_FRAMES,
        }),
    );
    refresh();
}

pub fn show_frame_advantage(frames: i32) {
    assign(
        &NUMBER,
        Some(NumberLine {
            number: Number::FrameAdvantage(frames),
            frames_left: NUMBER_FRAMES,
        }),
    );
    refresh();
}

pub fn show_hitstun_left(frames: i32) {
    assign(
        &NUMBER,
        Some(NumberLine {
            number: Number::HitstunLeft(frames),
            frames_left: NUMBER_FRAMES,
        }),
    );
    refresh();
}

/// Counts down what is on screen. Runs every frame, from the player's update.
pub unsafe fn tick(module_accessor: &mut BattleObjectModuleAccessor) {
    let entry_id = WorkModule::get_int(module_accessor, *FIGHTER_INSTANCE_WORK_ID_INT_ENTRY_ID);
    if entry_id != FighterId::Player as i32 {
        return;
    }

    let mut changed = false;
    if let Some(mut hitbox) = read_clone(&HITBOX) {
        hitbox.frames_left = hitbox.frames_left.saturating_sub(1);
        if hitbox.frames_left == 0 {
            assign(&HITBOX, None);
            changed = true;
        } else {
            assign(&HITBOX, Some(hitbox));
        }
    }
    if let Some(mut line) = read(&NUMBER) {
        line.frames_left = line.frames_left.saturating_sub(1);
        if line.frames_left == 0 {
            assign(&NUMBER, None);
            changed = true;
        } else {
            assign(&NUMBER, Some(line));
        }
    }
    // Only redraw when something disappeared, so the box is not rebuilt every frame
    if changed {
        refresh();
    }
}
