//! One command registry owns editor key chords, help copy and focus-sensitive action routing.

use super::monitoring::MonitoringAction;
use super::{Action, SettingsPage, WorkspaceAction, timeline::TimelineAction};
use recording_studio::{
    MediaTime,
    playback::monitoring::{JUMP_SECONDS, MonitoringSpeed},
};

macro_rules! keys {
    ($($key:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum Key { $($key),+ }
        impl Key {
            pub const ALL: &[Self] = &[$(Self::$key),+];
            pub const fn label(self) -> &'static str { match self { $(Self::$key => $label),+ } }
        }
    };
}
keys! { S => "S", N => "N", Z => "Z", Y => "Y", E => "E", R => "R", A => "A", D => "D", F => "F", G => "G", L => "L", BracketLeft => "[", BracketRight => "]", Space => "Space", Left => "Left", Right => "Right", Delete => "Delete", Plus => "+", Minus => "-", PageUp => "Page Up", PageDown => "Page Down", Home => "Home", End => "End", Help => "F1" }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Chord {
    Plain,
    Shift,
    Command,
    CommandShift,
}

impl Chord {
    /// Exact modifier matching avoids turning a modified text/navigation chord into another edit.
    pub fn matches(self, command: bool, shift: bool) -> bool {
        match self {
            Self::Plain => !command && !shift,
            Self::Shift => !command && shift,
            Self::Command => command && !shift,
            Self::CommandShift => command && shift,
        }
    }

    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Shift => "Shift+",
            Self::Command => "Ctrl/Cmd+",
            Self::CommandShift => "Ctrl/Cmd+Shift+",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Global,
    Project,
    Editor,
    Timeline,
    Inspection,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    pub project: bool,
    pub editor: bool,
    pub timeline: bool,
    pub camera_focused: bool,
    pub inspected: bool,
}

impl Scope {
    /// Timeline letters never consume focused camera movement; global commands remain accessible.
    pub fn available(self, context: Context) -> bool {
        match self {
            Self::Global => true,
            Self::Project => context.project,
            Self::Editor => context.editor,
            Self::Timeline => context.editor && context.timeline && !context.camera_focused,
            Self::Inspection => context.editor && context.inspected,
        }
    }
}

pub struct Spec {
    pub key: Key,
    pub chord: Chord,
    pub scope: Scope,
    pub description: &'static str,
}

macro_rules! bindings {
    (|$duration:ident| $($name:ident : $key:ident, $chord:ident, $scope:ident, $label:literal => $action:expr);+ $(;)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum Command { $($name),+ }
        impl Command {
            pub const ALL: &[Self] = &[$(Self::$name),+];
            /// Help and runtime matching read the same closed set of bindings.
            pub const fn spec(self) -> Spec { match self { $(Self::$name => Spec { key: Key::$key, chord: Chord::$chord, scope: Scope::$scope, description: $label }),+ } }
            /// Resolves a registered command into one typed editor intent.
            pub fn action(self, $duration: MediaTime) -> Action { match self { $(Self::$name => $action),+ } }
        }
    };
}

bindings! { |_duration|
    Save: S, Command, Project, "Save project" => Action::Save;
    New: N, Command, Global, "New project" => Action::NewProject;
    Undo: Z, Command, Project, "Undo" => Action::Undo;
    Redo: Z, CommandShift, Project, "Redo" => Action::Redo;
    RedoAlternate: Y, Command, Project, "Redo" => Action::Redo;
    Export: E, Command, Project, "Export video" => Action::Export;
    ResetView: R, Plain, Inspection, "Reset inspected camera" => Action::ResetCameraView;
    Play: Space, Plain, Editor, "Play / pause" => Action::TogglePlayback;
    PreviousFrame: Left, Plain, Timeline, "Previous frame" => Action::Step(-1);
    NextFrame: Right, Plain, Timeline, "Next frame" => Action::Step(1);
    BackTenFrames: Left, Shift, Timeline, "Back ten frames" => Action::Step(-10);
    ForwardTenFrames: Right, Shift, Timeline, "Forward ten frames" => Action::Step(10);
    Split: S, Plain, Timeline, "Split selected clip" => Action::Split;
    Delete: Delete, Plain, Timeline, "Delete selection" => Action::DeleteSelected;
    SelectAll: A, Command, Timeline, "Select all unlocked items" => Action::Timeline(TimelineAction::SelectAll);
    Duplicate: D, Command, Timeline, "Duplicate selection" => Action::Timeline(TimelineAction::Duplicate);
    Fit: F, Plain, Timeline, "Fit timeline" => Action::FitTimeline;
    ZoomIn: Plus, Plain, Timeline, "Zoom timeline in" => Action::Zoom(1);
    ZoomInAlternate: Plus, Shift, Timeline, "Zoom timeline in" => Action::Zoom(1);
    ZoomOut: Minus, Plain, Timeline, "Zoom timeline out" => Action::Zoom(-1);
    PanBack: PageUp, Plain, Timeline, "Pan timeline back" => Action::Pan(-1);
    PanForward: PageDown, Plain, Timeline, "Pan timeline forward" => Action::Pan(1);
    Start: Home, Plain, Editor, "Go to start" => Action::Monitoring(MonitoringAction::Start);
    End: End, Plain, Editor, "Go to end" => Action::Monitoring(MonitoringAction::End);
    JumpBack: Left, Command, Editor, "Jump back five seconds" => Action::Monitoring(MonitoringAction::Jump(-JUMP_SECONDS));
    JumpForward: Right, Command, Editor, "Jump forward five seconds" => Action::Monitoring(MonitoringAction::Jump(JUMP_SECONDS));
    Seek: G, Command, Editor, "Seek to seconds or timecode" => Action::Monitoring(MonitoringAction::EditSeek);
    Slower: BracketLeft, Plain, Editor, "Slower monitoring (audio pitch preserved)" => Action::Monitoring(MonitoringAction::StepSpeed(-1));
    Faster: BracketRight, Plain, Editor, "Faster monitoring (audio pitch preserved)" => Action::Monitoring(MonitoringAction::StepSpeed(1));
    NormalSpeed: BracketRight, Shift, Editor, "Reset monitoring to 1x" => Action::Monitoring(MonitoringAction::Speed(MonitoringSpeed::Normal));
    Loop: L, Plain, Editor, "Loop whole project (monitor only)" => Action::Monitoring(MonitoringAction::ToggleLoop);
    Help: Help, Plain, Global, "Show Studio controls" => Action::Workspace(WorkspaceAction::SettingsPage(SettingsPage::Controls));
}

/// The first exact available binding owns the key; matching is independent of UI rendering.
pub fn resolve(key: Key, command: bool, shift: bool, context: Context) -> Option<Command> {
    Command::ALL.iter().copied().find(|binding| {
        let spec = binding.spec();
        spec.key == key && spec.chord.matches(command, shift) && spec.scope.available(context)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fly_keys_cannot_split_or_select_timeline_items() {
        let context = Context {
            project: true,
            editor: true,
            timeline: true,
            camera_focused: true,
            ..Default::default()
        };
        assert_eq!(resolve(Key::S, false, false, context), None);
        assert_eq!(resolve(Key::F, false, false, context), None);
        assert_eq!(resolve(Key::S, true, false, context), Some(Command::Save));
        assert_eq!(
            resolve(Key::Space, false, false, context),
            Some(Command::Play)
        );
        let timeline = Context {
            camera_focused: false,
            ..context
        };
        assert_eq!(
            resolve(Key::S, false, false, timeline),
            Some(Command::Split)
        );
        assert_eq!(
            resolve(Key::Left, false, true, timeline)
                .unwrap()
                .action(MediaTime::ZERO),
            Action::Step(-10)
        );
    }
    #[test]
    fn help_registry_has_no_conflicting_chords_within_an_available_scope() {
        for (index, &command) in Command::ALL.iter().enumerate() {
            let spec = command.spec();
            assert!(!spec.description.is_empty());
            for &other in &Command::ALL[index + 1..] {
                let other = other.spec();
                assert!(spec.key != other.key || spec.chord != other.chord);
            }
        }
    }
}
