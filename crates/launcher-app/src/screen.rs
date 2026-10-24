//! The launcher window's state: what each screen shows and what each button does.

use launcher::install_layout::InstallLayout;
use update_manifest::channel::Channel;

/// Facts about this installation shown on the home screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Overview {
    /// The packaged client version.
    pub version: String,
    /// The saved update channel.
    pub channel: Channel,
    /// Whether the game resources are prepared; otherwise the client sets them up on Play.
    pub resources_ready: bool,
    /// Where the client keeps worlds, logs and settings.
    pub data_folder: String,
    /// A development checkout rather than a packaged install.
    pub development: bool,
}

impl Overview {
    /// How many rows [`Overview::rows`] returns.
    pub const ROWS: usize = 4;

    /// Reads the overview for `layout` with the update channel saved in its settings.
    pub fn read(layout: &InstallLayout, channel: Channel) -> Self {
        Self {
            version: launcher::PRODUCT_VERSION.to_owned(),
            channel,
            resources_ready: !first_run::needs_preparation(layout),
            data_folder: layout.user_data_root.display().to_string(),
            development: !layout.is_installed(),
        }
    }

    /// The labelled rows under the home screen's heading, in display order.
    pub fn rows(&self) -> [(&'static str, String); Self::ROWS] {
        [
            ("Version", self.version.clone()),
            (
                "Update channel",
                if self.development {
                    "Development build".to_owned()
                } else {
                    self.channel.label().to_owned()
                },
            ),
            (
                "Game resources",
                if self.resources_ready {
                    "Ready".to_owned()
                } else {
                    "Set up on first play".to_owned()
                },
            ),
            ("Data folder", self.data_folder.clone()),
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    Home,
    /// The client process has been started; the launcher closes shortly.
    Launching,
    Failed {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Play,
    OpenFolder,
    Quit,
}

/// What pressing a button asks the window to do besides changing screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    None,
    Launch,
    OpenFolder,
    Quit,
}

impl Screen {
    /// Buttons in display order; the first non-Quit one is the Enter default.
    pub const fn actions(&self) -> &'static [Action] {
        match self {
            Self::Home => &[Action::Play, Action::OpenFolder, Action::Quit],
            Self::Failed { .. } => &[Action::Play, Action::Quit],
            Self::Launching => &[],
        }
    }

    /// The highlighted default button, if the screen has one.
    pub fn primary(&self) -> Option<Action> {
        self.actions()
            .first()
            .copied()
            .filter(|action| *action != Action::Quit)
    }

    /// The next screen (if it changes) and the side effect of pressing `action`.
    pub fn on_action(&self, action: Action) -> (Option<Self>, Effect) {
        if !self.actions().contains(&action) {
            return (None, Effect::None);
        }
        match action {
            Action::Play => (Some(Self::Launching), Effect::Launch),
            Action::OpenFolder => (None, Effect::OpenFolder),
            Action::Quit => (None, Effect::Quit),
        }
    }

    /// The screen after the client process started or failed to start.
    pub fn launched(result: Result<(), String>) -> Self {
        match result {
            Ok(()) => Self::Launching,
            Err(message) => Self::Failed { message },
        }
    }

    pub const fn button_label(&self, action: Action) -> &'static str {
        match (action, self) {
            (Action::Play, Self::Failed { .. }) => "Try again",
            (Action::Play, _) => "Play",
            (Action::OpenFolder, _) => "Open data folder",
            (Action::Quit, _) => "Quit",
        }
    }

    pub const fn title(&self) -> &'static str {
        match self {
            Self::Home => "Ready to play",
            Self::Launching => concat!("Starting ", launcher::product_name!(), "…"),
            Self::Failed { .. } => concat!("Couldn't start ", launcher::product_name!()),
        }
    }

    /// Body text under the heading; empty when the screen shows rows instead.
    pub fn body(&self) -> String {
        match self {
            Self::Home => String::new(),
            Self::Launching => "The game is starting; this window closes in a moment.".to_owned(),
            Self::Failed { message } => message.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overview() -> Overview {
        Overview {
            version: "0.1.20".into(),
            channel: Channel::Beta,
            resources_ready: true,
            data_folder: "/data".into(),
            development: false,
        }
    }

    #[test]
    fn play_starts_the_client_and_failures_offer_another_try() {
        assert_eq!(Screen::Home.primary(), Some(Action::Play));
        assert_eq!(
            Screen::Home.on_action(Action::Play),
            (Some(Screen::Launching), Effect::Launch)
        );
        let failed = Screen::launched(Err("missing client".into()));
        assert_eq!(failed.button_label(Action::Play), "Try again");
        assert_eq!(
            failed.on_action(Action::Play),
            (Some(Screen::Launching), Effect::Launch)
        );
        assert_eq!(Screen::launched(Ok(())), Screen::Launching);
    }

    #[test]
    fn buttons_only_act_on_screens_that_show_them() {
        assert_eq!(
            Screen::Home.on_action(Action::OpenFolder),
            (None, Effect::OpenFolder)
        );
        assert_eq!(Screen::Home.on_action(Action::Quit), (None, Effect::Quit));
        assert!(Screen::Launching.actions().is_empty());
        assert_eq!(
            Screen::Launching.on_action(Action::Play),
            (None, Effect::None)
        );
        let failed = Screen::Failed {
            message: String::new(),
        };
        assert_eq!(failed.on_action(Action::OpenFolder), (None, Effect::None));
    }

    #[test]
    fn rows_describe_channel_and_resources() {
        let rows = overview().rows();
        assert_eq!(rows[1].1, Channel::Beta.label());
        assert_eq!(rows[2].1, "Ready");
        let pending = Overview {
            resources_ready: false,
            development: true,
            ..overview()
        };
        let rows = pending.rows();
        assert_eq!(rows[1].1, "Development build");
        assert_eq!(rows[2].1, "Set up on first play");
    }
}
