//! Maya's own resident session as a board card.

use crate::AppState;
use maya_core::model::Card;

/// The brain's card, appended to the board the window sees. None until the
/// brain session exists.
pub(crate) fn card(_state: &AppState) -> Option<Card> {
    None
}
