//! Account changes cancel the original native session ownership before storage work.

use super::*;

/// Runs before menu services can dispatch signout or begin its original account storage job.
pub(crate) fn before_menu_services(mut menu: ResMut<MenuRuntime>, mut session: SessionResources) {
    if !menu.account_operation_pending() {
        session.controller.account_retirement_started = false;
        return;
    }
    if session.controller.account_retirement_started {
        return;
    }
    session.leave();
    session.controller.trust = None;
    #[cfg(feature = "native-networking")]
    {
        session.controller.native_account = None;
    }
    session.controller.account_retirement_started = true;
}
