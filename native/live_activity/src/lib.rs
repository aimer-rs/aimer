#![doc = include_str!("../README.md")]

mod activity;
mod backend;
mod center;
mod payload;
mod view;

pub use activity::{LiveActivity, LiveActivityId, Milestone, Progress};
pub use center::{LiveActivities, LiveActivitiesBuilder, LiveActivityError, Presentation};
pub use view::{
    Bar, Clock, Column, Gap, Label, Region, Row, Symbol, TextStyle, Tint, UIKitWidget, ViewNode,
};
