//! One file per view. Each page in the sidebar (see `nav`) has a view here and
//! a store of the same name in `state/`, owned together by that page.

pub mod albums;
pub mod artist;
pub mod concerts;
pub mod eras;
pub mod history;
pub mod nav;
pub mod now_playing;
pub mod on_repeat;
pub mod on_this_day;
pub mod overview;
pub mod palette;
pub mod play_detail;
pub mod rhythm;
pub mod root;
pub mod setup;
pub mod shell;
pub mod top_over_time;
pub mod track;
pub mod widgets;
pub mod wrapped;
