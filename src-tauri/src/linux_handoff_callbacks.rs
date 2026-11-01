//! Parse shortcut invocations forwarded by supported Linux launchers.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Callback {
    Release,
    Handsfree,
    CaptureSubApp,
    Show,
}

impl Callback {
    pub(crate) fn from_args(args: &[String]) -> Self {
        if args.iter().any(|arg| arg == "--verenu-hotkey-release") {
            Self::Release
        } else if args.iter().any(|arg| arg == "--verenu-hotkey-handsfree") {
            Self::Handsfree
        } else if args.iter().any(|arg| arg == "--verenu-capture-sub-app") {
            Self::CaptureSubApp
        } else {
            Self::Show
        }
    }
}
