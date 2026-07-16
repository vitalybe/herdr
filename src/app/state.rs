use crate::config::{Keybinds, NewTerminalCwdConfig, SoundConfig, ToastConfig, ToastDelivery};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Direction, Rect};
use ratatui::style::Color;

use crate::detect::AgentState;
use crate::layout::{PaneId, PaneInfo, SplitBorder};
use crate::selection::Selection;

pub(crate) type InstalledPluginRegistry =
    std::collections::HashMap<String, crate::api::schema::InstalledPluginInfo>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PluginPaneRecord {
    pub plugin_id: String,
    pub entrypoint: String,
}

// ---------------------------------------------------------------------------
// Selection autoscroll types
// ---------------------------------------------------------------------------

/// Direction of automatic scrolling during text selection drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionAutoscrollDirection {
    Up,
    Down,
}

/// State for automatic scrolling during text selection drag.
///
/// When the cursor hovers in the 1-row hot zone at the top or bottom edge
/// of a pane (or outside the pane), this struct captures the direction and
/// last known mouse position so a recurring 30ms tick can continue scrolling
/// and extending the selection even when the mouse is not moving.
#[derive(Clone, Debug)]
pub(crate) struct SelectionAutoscroll {
    pub direction: SelectionAutoscrollDirection,
    pub last_mouse_screen_col: u16,
    pub last_mouse_screen_row: u16,
    pub inner_rect: Rect,
}

#[derive(Clone)]
pub(crate) struct RightClickPassthroughGesture {
    pub pane_info: PaneInfo,
    pub modifiers: KeyModifiers,
}
use crate::terminal_theme::{HostAppearance, TerminalTheme};
use crate::workspace::Workspace;

// ---------------------------------------------------------------------------
// Theme palette — all UI colors in one place, ready for theming
// ---------------------------------------------------------------------------

/// All colors used by the UI. Derived from a base accent color for now,
/// but structured so a full theme system can replace it later.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // all fields defined for theming — some used later
pub struct Palette {
    /// Primary accent (highlight, active borders).
    pub accent: Color,
    /// Background for floating panels, overlays, and modals.
    pub panel_bg: Color,
    /// Subtle surface background for selected/focused items.
    pub surface0: Color,
    /// Slightly lighter surface for hover/active states.
    pub surface1: Color,
    /// Very dim surface for separators.
    pub surface_dim: Color,
    /// Muted text (secondary info, numbers).
    pub overlay0: Color,
    /// Slightly brighter overlay text.
    pub overlay1: Color,
    /// Main text color — soft white.
    pub text: Color,
    /// Subdued text (workspace numbers, dim labels).
    pub subtext0: Color,
    /// Branch name / special label color.
    pub mauve: Color,
    /// Done / idle states.
    pub green: Color,
    /// Working / running states.
    pub yellow: Color,
    /// Needs attention / blocked states.
    pub red: Color,
    /// Unseen / done notification accent.
    pub blue: Color,
    /// Notification accent / unseen markers.
    pub teal: Color,
    /// Interrupted / warning states.
    pub peach: Color,
}

impl Palette {
    /// Catppuccin Mocha — the default.
    pub fn catppuccin() -> Self {
        Self {
            accent: Color::Rgb(137, 180, 250), // blue
            panel_bg: Color::Rgb(24, 24, 37),
            surface0: Color::Rgb(49, 50, 68),
            surface1: Color::Rgb(69, 71, 90),
            surface_dim: Color::Rgb(30, 30, 46),
            overlay0: Color::Rgb(108, 112, 134),
            overlay1: Color::Rgb(127, 132, 156),
            text: Color::Rgb(205, 214, 244),
            subtext0: Color::Rgb(166, 173, 200),
            mauve: Color::Rgb(203, 166, 247),
            green: Color::Rgb(166, 227, 161),
            yellow: Color::Rgb(249, 226, 175),
            red: Color::Rgb(243, 139, 168),
            blue: Color::Rgb(137, 180, 250),
            teal: Color::Rgb(148, 226, 213),
            peach: Color::Rgb(250, 179, 135),
        }
    }

    /// Catppuccin Latte — the light Catppuccin flavor.
    pub fn catppuccin_latte() -> Self {
        Self {
            accent: Color::Rgb(30, 102, 245),
            panel_bg: Color::Rgb(239, 241, 245),
            surface0: Color::Rgb(204, 208, 218),
            surface1: Color::Rgb(188, 192, 204),
            surface_dim: Color::Rgb(230, 233, 239),
            overlay0: Color::Rgb(156, 160, 176),
            overlay1: Color::Rgb(140, 143, 161),
            text: Color::Rgb(76, 79, 105),
            subtext0: Color::Rgb(108, 111, 133),
            mauve: Color::Rgb(136, 57, 239),
            green: Color::Rgb(64, 160, 43),
            yellow: Color::Rgb(223, 142, 29),
            red: Color::Rgb(210, 15, 57),
            blue: Color::Rgb(30, 102, 245),
            teal: Color::Rgb(23, 146, 153),
            peach: Color::Rgb(254, 100, 11),
        }
    }

    /// Terminal 16-color theme.
    pub fn terminal() -> Self {
        Self {
            accent: Color::Blue,
            panel_bg: Color::Reset,
            surface0: Color::Reset,
            surface1: Color::DarkGray,
            surface_dim: Color::DarkGray,
            overlay0: Color::Gray,
            overlay1: Color::White,
            text: Color::Reset,
            subtext0: Color::Gray,
            mauve: Color::Gray,
            green: Color::Green,
            yellow: Color::Yellow,
            red: Color::LightRed,
            blue: Color::Blue,
            teal: Color::Cyan,
            peach: Color::Yellow,
        }
    }

    /// Tokyo Night — blue-purple aesthetic.
    pub fn tokyo_night() -> Self {
        Self {
            accent: Color::Rgb(122, 162, 247), // blue
            panel_bg: Color::Rgb(26, 27, 38),
            surface0: Color::Rgb(36, 40, 59),
            surface1: Color::Rgb(65, 72, 104),
            surface_dim: Color::Rgb(26, 27, 38),
            overlay0: Color::Rgb(86, 95, 137),
            overlay1: Color::Rgb(105, 113, 150),
            text: Color::Rgb(192, 202, 245),
            subtext0: Color::Rgb(169, 177, 214),
            mauve: Color::Rgb(187, 154, 247),
            green: Color::Rgb(158, 206, 106),
            yellow: Color::Rgb(224, 175, 104),
            red: Color::Rgb(247, 118, 142),
            blue: Color::Rgb(122, 162, 247),
            teal: Color::Rgb(125, 207, 255),
            peach: Color::Rgb(255, 158, 100),
        }
    }

    /// Tokyo Night Day — the light Tokyo Night style.
    pub fn tokyo_night_day() -> Self {
        Self {
            accent: Color::Rgb(46, 125, 233),
            panel_bg: Color::Rgb(225, 226, 231),
            surface0: Color::Rgb(196, 200, 218),
            surface1: Color::Rgb(168, 174, 203),
            surface_dim: Color::Rgb(210, 211, 218),
            overlay0: Color::Rgb(137, 144, 179),
            overlay1: Color::Rgb(104, 112, 154),
            text: Color::Rgb(55, 96, 191),
            subtext0: Color::Rgb(97, 114, 176),
            mauve: Color::Rgb(120, 71, 189),
            green: Color::Rgb(88, 117, 57),
            yellow: Color::Rgb(140, 108, 62),
            red: Color::Rgb(245, 42, 101),
            blue: Color::Rgb(46, 125, 233),
            teal: Color::Rgb(17, 140, 116),
            peach: Color::Rgb(177, 92, 0),
        }
    }

    /// Dracula — purple/pink/green.
    pub fn dracula() -> Self {
        Self {
            accent: Color::Rgb(189, 147, 249), // purple
            panel_bg: Color::Rgb(40, 42, 54),
            surface0: Color::Rgb(68, 71, 90),
            surface1: Color::Rgb(98, 114, 164),
            surface_dim: Color::Rgb(40, 42, 54),
            overlay0: Color::Rgb(98, 114, 164),
            overlay1: Color::Rgb(130, 140, 180),
            text: Color::Rgb(248, 248, 242),
            subtext0: Color::Rgb(210, 210, 220),
            mauve: Color::Rgb(255, 121, 198), // pink
            green: Color::Rgb(80, 250, 123),
            yellow: Color::Rgb(241, 250, 140),
            red: Color::Rgb(255, 85, 85),
            blue: Color::Rgb(139, 233, 253), // cyan-ish
            teal: Color::Rgb(139, 233, 253),
            peach: Color::Rgb(255, 184, 108),
        }
    }

    /// Nord — frosty blue palette.
    pub fn nord() -> Self {
        Self {
            accent: Color::Rgb(136, 192, 208), // frost
            panel_bg: Color::Rgb(46, 52, 64),
            surface0: Color::Rgb(59, 66, 82),
            surface1: Color::Rgb(67, 76, 94),
            surface_dim: Color::Rgb(46, 52, 64),
            overlay0: Color::Rgb(76, 86, 106),
            overlay1: Color::Rgb(100, 110, 130),
            text: Color::Rgb(236, 239, 244),
            subtext0: Color::Rgb(216, 222, 233),
            mauve: Color::Rgb(180, 142, 173),
            green: Color::Rgb(163, 190, 140),
            yellow: Color::Rgb(235, 203, 139),
            red: Color::Rgb(191, 97, 106),
            blue: Color::Rgb(129, 161, 193),
            teal: Color::Rgb(143, 188, 187),
            peach: Color::Rgb(208, 135, 112),
        }
    }

    /// Gruvbox Dark — warm retro palette.
    pub fn gruvbox() -> Self {
        Self {
            accent: Color::Rgb(215, 153, 33), // yellow
            panel_bg: Color::Rgb(40, 40, 40),
            surface0: Color::Rgb(60, 56, 54),
            surface1: Color::Rgb(80, 73, 69),
            surface_dim: Color::Rgb(40, 40, 40),
            overlay0: Color::Rgb(146, 131, 116),
            overlay1: Color::Rgb(168, 153, 132),
            text: Color::Rgb(235, 219, 178),
            subtext0: Color::Rgb(213, 196, 161),
            mauve: Color::Rgb(211, 134, 155),
            green: Color::Rgb(184, 187, 38),
            yellow: Color::Rgb(250, 189, 47),
            red: Color::Rgb(251, 73, 52),
            blue: Color::Rgb(131, 165, 152),
            teal: Color::Rgb(142, 192, 124),
            peach: Color::Rgb(254, 128, 25),
        }
    }

    /// Gruvbox Light — the light retro palette.
    pub fn gruvbox_light() -> Self {
        Self {
            accent: Color::Rgb(7, 102, 120),
            panel_bg: Color::Rgb(251, 241, 199),
            surface0: Color::Rgb(235, 219, 178),
            surface1: Color::Rgb(213, 196, 161),
            surface_dim: Color::Rgb(242, 229, 188),
            overlay0: Color::Rgb(146, 131, 116),
            overlay1: Color::Rgb(124, 111, 100),
            text: Color::Rgb(60, 56, 54),
            subtext0: Color::Rgb(80, 73, 69),
            mauve: Color::Rgb(143, 63, 113),
            green: Color::Rgb(121, 116, 14),
            yellow: Color::Rgb(181, 118, 20),
            red: Color::Rgb(157, 0, 6),
            blue: Color::Rgb(7, 102, 120),
            teal: Color::Rgb(66, 123, 88),
            peach: Color::Rgb(175, 58, 3),
        }
    }

    /// One Dark — Atom's classic dark theme.
    pub fn one_dark() -> Self {
        Self {
            accent: Color::Rgb(97, 175, 239), // blue
            panel_bg: Color::Rgb(40, 44, 52),
            surface0: Color::Rgb(44, 49, 58),
            surface1: Color::Rgb(62, 68, 81),
            surface_dim: Color::Rgb(40, 44, 52),
            overlay0: Color::Rgb(92, 99, 112),
            overlay1: Color::Rgb(115, 122, 135),
            text: Color::Rgb(171, 178, 191),
            subtext0: Color::Rgb(150, 156, 168),
            mauve: Color::Rgb(198, 120, 221),
            green: Color::Rgb(152, 195, 121),
            yellow: Color::Rgb(229, 192, 123),
            red: Color::Rgb(224, 108, 117),
            blue: Color::Rgb(97, 175, 239),
            teal: Color::Rgb(86, 182, 194),
            peach: Color::Rgb(209, 154, 102),
        }
    }

    /// One Light — Atom's classic light theme.
    pub fn one_light() -> Self {
        Self {
            accent: Color::Rgb(64, 120, 242),
            panel_bg: Color::Rgb(250, 250, 250),
            surface0: Color::Rgb(240, 240, 241),
            surface1: Color::Rgb(229, 229, 230),
            surface_dim: Color::Rgb(245, 245, 246),
            overlay0: Color::Rgb(160, 161, 167),
            overlay1: Color::Rgb(104, 107, 119),
            text: Color::Rgb(56, 58, 66),
            subtext0: Color::Rgb(104, 107, 119),
            mauve: Color::Rgb(166, 38, 164),
            green: Color::Rgb(80, 161, 79),
            yellow: Color::Rgb(193, 132, 1),
            red: Color::Rgb(228, 86, 73),
            blue: Color::Rgb(64, 120, 242),
            teal: Color::Rgb(1, 132, 188),
            peach: Color::Rgb(152, 104, 1),
        }
    }

    /// Solarized Dark — Ethan Schoonover's classic.
    pub fn solarized() -> Self {
        Self {
            accent: Color::Rgb(38, 139, 210), // blue
            panel_bg: Color::Rgb(0, 43, 54),
            surface0: Color::Rgb(7, 54, 66),
            surface1: Color::Rgb(88, 110, 117),
            surface_dim: Color::Rgb(0, 43, 54),
            overlay0: Color::Rgb(88, 110, 117),
            overlay1: Color::Rgb(101, 123, 131),
            text: Color::Rgb(147, 161, 161),
            subtext0: Color::Rgb(131, 148, 150),
            mauve: Color::Rgb(211, 54, 130),
            green: Color::Rgb(133, 153, 0),
            yellow: Color::Rgb(181, 137, 0),
            red: Color::Rgb(220, 50, 47),
            blue: Color::Rgb(38, 139, 210),
            teal: Color::Rgb(42, 161, 152),
            peach: Color::Rgb(203, 75, 22),
        }
    }

    /// Solarized Light — Ethan Schoonover's light variant.
    pub fn solarized_light() -> Self {
        Self {
            accent: Color::Rgb(38, 139, 210),
            panel_bg: Color::Rgb(253, 246, 227),
            surface0: Color::Rgb(238, 232, 213),
            surface1: Color::Rgb(147, 161, 161),
            surface_dim: Color::Rgb(238, 232, 213),
            overlay0: Color::Rgb(147, 161, 161),
            overlay1: Color::Rgb(88, 110, 117),
            text: Color::Rgb(101, 123, 131),
            subtext0: Color::Rgb(131, 148, 150),
            mauve: Color::Rgb(211, 54, 130),
            green: Color::Rgb(133, 153, 0),
            yellow: Color::Rgb(181, 137, 0),
            red: Color::Rgb(220, 50, 47),
            blue: Color::Rgb(38, 139, 210),
            teal: Color::Rgb(42, 161, 152),
            peach: Color::Rgb(203, 75, 22),
        }
    }

    /// Kanagawa — inspired by Katsushika Hokusai.
    pub fn kanagawa() -> Self {
        Self {
            accent: Color::Rgb(126, 156, 216), // blue
            panel_bg: Color::Rgb(31, 31, 40),
            surface0: Color::Rgb(42, 42, 55),
            surface1: Color::Rgb(54, 54, 70),
            surface_dim: Color::Rgb(31, 31, 40),
            overlay0: Color::Rgb(114, 113, 105),
            overlay1: Color::Rgb(135, 134, 125),
            text: Color::Rgb(220, 215, 186),
            subtext0: Color::Rgb(200, 195, 170),
            mauve: Color::Rgb(149, 127, 184),
            green: Color::Rgb(118, 148, 106),
            yellow: Color::Rgb(192, 163, 110),
            red: Color::Rgb(195, 64, 67),
            blue: Color::Rgb(126, 156, 216),
            teal: Color::Rgb(127, 180, 202),
            peach: Color::Rgb(255, 160, 102),
        }
    }

    /// Kanagawa Lotus — the light Kanagawa variant.
    pub fn kanagawa_lotus() -> Self {
        Self {
            accent: Color::Rgb(77, 105, 155),
            panel_bg: Color::Rgb(242, 236, 188),
            surface0: Color::Rgb(220, 213, 172),
            surface1: Color::Rgb(201, 203, 209),
            surface_dim: Color::Rgb(213, 206, 163),
            overlay0: Color::Rgb(160, 156, 172),
            overlay1: Color::Rgb(138, 137, 128),
            text: Color::Rgb(84, 84, 100),
            subtext0: Color::Rgb(67, 67, 108),
            mauve: Color::Rgb(98, 76, 131),
            green: Color::Rgb(111, 137, 78),
            yellow: Color::Rgb(119, 113, 63),
            red: Color::Rgb(200, 64, 83),
            blue: Color::Rgb(77, 105, 155),
            teal: Color::Rgb(78, 140, 162),
            peach: Color::Rgb(204, 109, 0),
        }
    }

    /// Rosé Pine — muted, elegant.
    pub fn rose_pine() -> Self {
        Self {
            accent: Color::Rgb(196, 167, 231), // iris
            panel_bg: Color::Rgb(25, 23, 36),
            surface0: Color::Rgb(31, 29, 46),
            surface1: Color::Rgb(38, 35, 58),
            surface_dim: Color::Rgb(25, 23, 36),
            overlay0: Color::Rgb(110, 106, 134),
            overlay1: Color::Rgb(144, 140, 170),
            text: Color::Rgb(224, 222, 244),
            subtext0: Color::Rgb(200, 197, 220),
            mauve: Color::Rgb(196, 167, 231),  // iris
            green: Color::Rgb(49, 116, 143),   // pine
            yellow: Color::Rgb(246, 193, 119), // gold
            red: Color::Rgb(235, 111, 146),    // love
            blue: Color::Rgb(49, 116, 143),    // pine
            teal: Color::Rgb(156, 207, 216),   // foam
            peach: Color::Rgb(234, 154, 151),  // rose
        }
    }

    /// Rosé Pine Dawn — the light Rosé Pine variant.
    pub fn rose_pine_dawn() -> Self {
        Self {
            accent: Color::Rgb(144, 122, 169),
            panel_bg: Color::Rgb(250, 244, 237),
            surface0: Color::Rgb(242, 233, 225),
            surface1: Color::Rgb(255, 250, 243),
            surface_dim: Color::Rgb(242, 233, 225),
            overlay0: Color::Rgb(152, 147, 165),
            overlay1: Color::Rgb(121, 117, 147),
            text: Color::Rgb(70, 66, 97),
            subtext0: Color::Rgb(121, 117, 147),
            mauve: Color::Rgb(144, 122, 169),
            green: Color::Rgb(40, 105, 131),
            yellow: Color::Rgb(234, 157, 52),
            red: Color::Rgb(180, 99, 122),
            blue: Color::Rgb(40, 105, 131),
            teal: Color::Rgb(86, 148, 159),
            peach: Color::Rgb(215, 130, 126),
        }
    }

    /// Vesper — minimal high-contrast monochrome with peach and mint accents.
    pub fn vesper() -> Self {
        Self {
            accent: Color::Rgb(255, 199, 153),
            panel_bg: Color::Rgb(26, 26, 26),
            surface0: Color::Rgb(35, 35, 35),
            surface1: Color::Rgb(40, 40, 40),
            surface_dim: Color::Rgb(16, 16, 16),
            overlay0: Color::Rgb(92, 92, 92),
            overlay1: Color::Rgb(126, 126, 126),
            text: Color::Rgb(255, 255, 255),
            subtext0: Color::Rgb(160, 160, 160),
            mauve: Color::Rgb(255, 209, 168),
            green: Color::Rgb(153, 255, 228),
            yellow: Color::Rgb(255, 199, 153),
            red: Color::Rgb(255, 128, 128),
            blue: Color::Rgb(176, 176, 176),
            teal: Color::Rgb(102, 221, 204),
            peach: Color::Rgb(255, 199, 153),
        }
    }

    /// Resolve a theme by name. Returns None for unknown names.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().replace([' ', '_'], "-").as_str() {
            "catppuccin" | "catppuccin-mocha" => Some(Self::catppuccin()),
            "catppuccin-latte" | "latte" | "light" => Some(Self::catppuccin_latte()),
            "terminal" => Some(Self::terminal()),
            "tokyo-night" | "tokyonight" => Some(Self::tokyo_night()),
            "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => Some(Self::tokyo_night_day()),
            "dracula" => Some(Self::dracula()),
            "nord" => Some(Self::nord()),
            "gruvbox" | "gruvbox-dark" => Some(Self::gruvbox()),
            "gruvbox-light" => Some(Self::gruvbox_light()),
            "one-dark" | "onedark" => Some(Self::one_dark()),
            "one-light" | "onelight" => Some(Self::one_light()),
            "solarized" | "solarized-dark" => Some(Self::solarized()),
            "solarized-light" => Some(Self::solarized_light()),
            "kanagawa" => Some(Self::kanagawa()),
            "kanagawa-lotus" | "lotus" => Some(Self::kanagawa_lotus()),
            "rose-pine" | "rosepine" => Some(Self::rose_pine()),
            "rose-pine-dawn" | "rosepine-dawn" | "dawn" => Some(Self::rose_pine_dawn()),
            "vesper" => Some(Self::vesper()),
            _ => None,
        }
    }

    /// Apply custom color overrides on top of this palette.
    pub fn with_overrides(mut self, custom: &crate::config::CustomThemeColors) -> Self {
        use crate::config::parse_color;
        if let Some(c) = &custom.accent {
            self.accent = parse_color(c);
        }
        if let Some(c) = &custom.panel_bg {
            self.panel_bg = parse_color(c);
        }
        if let Some(c) = &custom.surface0 {
            self.surface0 = parse_color(c);
        }
        if let Some(c) = &custom.surface1 {
            self.surface1 = parse_color(c);
        }
        if let Some(c) = &custom.surface_dim {
            self.surface_dim = parse_color(c);
        }
        if let Some(c) = &custom.overlay0 {
            self.overlay0 = parse_color(c);
        }
        if let Some(c) = &custom.overlay1 {
            self.overlay1 = parse_color(c);
        }
        if let Some(c) = &custom.text {
            self.text = parse_color(c);
        }
        if let Some(c) = &custom.subtext0 {
            self.subtext0 = parse_color(c);
        }
        if let Some(c) = &custom.mauve {
            self.mauve = parse_color(c);
        }
        if let Some(c) = &custom.green {
            self.green = parse_color(c);
        }
        if let Some(c) = &custom.yellow {
            self.yellow = parse_color(c);
        }
        if let Some(c) = &custom.red {
            self.red = parse_color(c);
        }
        if let Some(c) = &custom.blue {
            self.blue = parse_color(c);
        }
        if let Some(c) = &custom.teal {
            self.teal = parse_color(c);
        }
        if let Some(c) = &custom.peach {
            self.peach = parse_color(c);
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceCardArea {
    pub ws_idx: usize,
    pub rect: Rect,
    pub indented: bool,
}

/// Content of a visible Panes-section row: a non-agent pane (two lines) or a
/// named line-split divider (one line). Client-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneSectionRowContent {
    Pane {
        ws_idx: usize,
        tab_idx: usize,
        pane_id: crate::layout::PaneId,
    },
    LineSplit {
        id: LineSplitId,
    },
}

/// Screen placement of one visible Panes-section row. `order_idx` is the flat
/// index into `PaneSectionOrder`; `content` carries whether the row is a pane
/// (with its `ws_idx`/`tab_idx`/`pane_id` for focus and rename) or a line-split.
/// Client-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSectionRowArea {
    pub order_idx: usize,
    pub content: PaneSectionRowContent,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCreateState {
    pub source_workspace_id: String,
    pub source_checkout_path: std::path::PathBuf,
    pub source_existing_membership: Option<crate::workspace::WorktreeSpaceMembership>,
    pub source_repo_root: std::path::PathBuf,
    pub repo_key: String,
    pub repo_name: String,
    pub branch: String,
    pub checkout_path: std::path::PathBuf,
    pub error: Option<String>,
    pub creating: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRemoveState {
    pub workspace_id: String,
    pub repo_root: std::path::PathBuf,
    pub path: std::path::PathBuf,
    pub error: Option<String>,
    pub removing: bool,
    pub force_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeOpenEntry {
    pub path: std::path::PathBuf,
    pub branch: Option<String>,
    pub is_linked_worktree: bool,
    pub already_open_ws_idx: Option<usize>,
}

impl WorktreeOpenEntry {
    pub(crate) fn display_name(&self) -> String {
        self.branch.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }

    pub(crate) fn status_label(&self) -> &'static str {
        if self.already_open_ws_idx.is_some() {
            "open"
        } else if self.branch.is_some() {
            ""
        } else if self.is_linked_worktree {
            "detached"
        } else {
            "root"
        }
    }

    fn search_text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.display_name(),
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
            self.path.display(),
            self.status_label()
        )
        .to_lowercase()
    }

    fn matches_query(&self, query: &str) -> bool {
        text_matches_query(query, &self.search_text())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeOpenState {
    pub source_workspace_id: String,
    pub source_existing_membership: Option<crate::workspace::WorktreeSpaceMembership>,
    pub source_checkout_path: std::path::PathBuf,
    pub source_repo_root: std::path::PathBuf,
    pub repo_key: String,
    pub repo_name: String,
    pub entries: Vec<WorktreeOpenEntry>,
    pub selected: usize,
    pub query: String,
    pub search_focused: bool,
    pub error: Option<String>,
}

impl WorktreeOpenState {
    pub(crate) fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim();
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                (query.is_empty() || entry.matches_query(query)).then_some(idx)
            })
            .collect()
    }

    pub(crate) fn selected_entry_index(&self) -> Option<usize> {
        let indices = self.filtered_indices();
        if indices.contains(&self.selected) {
            Some(self.selected)
        } else {
            indices.first().copied()
        }
    }

    pub(crate) fn normalize_selection(&mut self) {
        if let Some(selected) = self.selected_entry_index() {
            self.selected = selected;
        }
    }

    pub(crate) fn select_previous_filtered(&mut self) {
        let indices = self.filtered_indices();
        let Some(current) = self.selected_entry_index() else {
            return;
        };
        let pos = indices.iter().position(|idx| *idx == current).unwrap_or(0);
        self.selected = indices[pos.saturating_sub(1)];
    }

    pub(crate) fn select_next_filtered(&mut self) {
        let indices = self.filtered_indices();
        let Some(current) = self.selected_entry_index() else {
            return;
        };
        let pos = indices.iter().position(|idx| *idx == current).unwrap_or(0);
        self.selected = indices[(pos + 1).min(indices.len().saturating_sub(1))];
    }
}

pub(crate) fn text_matches_query(query: &str, text: &str) -> bool {
    let haystack = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|needle| haystack.contains(needle))
}

/// Computed view geometry — derived from AppState + terminal size.
/// Updated before each render, consumed by render and mouse handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewLayout {
    Desktop,
    Mobile,
}

pub struct ViewState {
    pub layout: ViewLayout,
    pub sidebar_rect: Rect,
    pub workspace_card_areas: Vec<WorkspaceCardArea>,
    pub pane_section_row_areas: Vec<PaneSectionRowArea>,
    pub tab_bar_rect: Rect,
    pub tab_hit_areas: Vec<Rect>,
    pub tab_scroll_left_hit_area: Rect,
    pub tab_scroll_right_hit_area: Rect,
    pub new_tab_hit_area: Rect,
    pub terminal_area: Rect,
    pub mobile_header_rect: Rect,
    pub mobile_menu_hit_area: Rect,
    pub toast_hit_area: Rect,
    pub pane_infos: Vec<PaneInfo>,
    pub split_borders: Vec<SplitBorder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Onboarding,
    ReleaseNotes,
    ProductAnnouncement,
    Navigate,
    Prefix,
    Copy,
    Terminal,
    RenameWorkspace,
    RenameTab,
    RenamePane,
    RenameAgent,
    RenameLineSplit,
    NewLinkedWorktree,
    OpenExistingWorktree,
    ConfirmRemoveWorktree,
    Resize,
    ConfirmClose,
    ConfirmAgentReparent,
    ContextMenu,
    Settings,
    GlobalMenu,
    KeybindHelp,
    Navigator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NavigatorTarget {
    Workspace {
        ws_idx: usize,
    },
    Tab {
        ws_idx: usize,
        tab_idx: usize,
    },
    Pane {
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigatorRow {
    pub target: NavigatorTarget,
    pub depth: u8,
    pub label: String,
    pub meta: String,
    pub status: AgentState,
    pub seen: bool,
    pub is_current: bool,
    pub is_workspace: bool,
    pub is_tab: bool,
    pub expanded: bool,
    pub search_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigatorStateFilter {
    Blocked,
    Working,
    Idle,
    Done,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NavigatorState {
    pub query: String,
    pub selected: usize,
    pub scroll: usize,
    pub search_focused: bool,
    pub state_filter: Option<NavigatorStateFilter>,
    pub expanded_workspaces: std::collections::HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CopyModeState {
    pub pane_id: PaneId,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub entry_offset_from_bottom: usize,
    pub selection: Option<CopyModeSelection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyModeSelection {
    Character,
    Linewise { anchor_row: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopySearchDirection {
    Forward,
    Backward,
}

/// A single scrollback search hit, addressed by absolute row and the cell
/// column span it occupies (matching the row/cell model in
/// [`crate::selection::Selection`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CopySearchMatch {
    pub row: u32,
    pub col: u16,
    pub width: u16,
}

/// Scrollback find state that rides alongside copy mode. Kept separate from
/// [`CopyModeState`] so that value stays `Copy`; this owns the query string and
/// match list, which cannot be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopySearchState {
    pub query: String,
    pub direction: CopySearchDirection,
    /// True while the prompt is focused and the query is being edited. False
    /// once the search is committed and the user is stepping matches.
    pub editing: bool,
    pub matches: Vec<CopySearchMatch>,
    pub current: Option<usize>,
    /// Scroll offset captured when the search opened, restored on cancel.
    pub origin_offset_from_bottom: usize,
    /// Cursor position (absolute row, cell column) when the search opened.
    /// Incremental matches are chosen relative to this anchor, like vim's
    /// incremental search, so growing the query does not drift the anchor.
    pub origin_row: u32,
    pub origin_col: u16,
    /// True when the search opened directly (via the `find` action) rather than
    /// from within an existing copy-mode session. Cancelling a direct search
    /// leaves copy mode entirely; cancelling a search started from copy mode
    /// stays in copy mode.
    pub entered_via_find: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentPanelSort {
    #[default]
    Spaces,
    Priority,
    Manual,
}

/// Client-only identifier for a named line-split divider row in the agents
/// panel. A monotonic counter (see [`AgentManualOrder::next_line_split_id`])
/// hands these out so ordering, drag, rename, and persistence are all
/// deterministic and testable. Never enters the server/runtime protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LineSplitId(pub(crate) u64);

/// Which independent sidebar section a line-split belongs to. The agents panel
/// and the Panes section each own their own flat order and their own monotonic
/// [`LineSplitId`] counter, so ids can collide across sections; this
/// discriminator keeps the shared rename modal and context menu routed to the
/// right list. Client-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineSplitSection {
    Agents,
    Panes,
}

/// A single entry in the manual agent ordering. Either an agent pane or a
/// user-created named line-split divider, interleaved in one flat vector.
#[derive(Debug, Clone)]
pub(crate) enum ManualEntry {
    Pane(PaneId),
    LineSplit { id: LineSplitId, name: String },
}

/// Reference to a single manual-order entry, used to pick up and move either an
/// agent pane or a line-split during drag-and-drop. Client-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManualEntryRef {
    Pane(PaneId),
    LineSplit(LineSplitId),
}

/// What a pending drag-to-reparent operation will do once confirmed. Attach the
/// dragged agent under a parent, or detach it back to the top level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentReparentAction {
    SetParent {
        parent_ws: usize,
        parent_pane: PaneId,
    },
    ClearParent,
}

/// A drag-to-reparent operation awaiting confirmation in the
/// [`Mode::ConfirmAgentReparent`] modal. Client-only presentation state; the
/// resolved parent link it produces is the only runtime fact and is applied on
/// confirm. Ephemeral, never persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingAgentReparent {
    pub child_ws: usize,
    pub child_pane: PaneId,
    /// Display label of the dragged agent, for the modal prompt.
    pub child_label: String,
    /// Display label of the target parent (SetParent) or the current parent
    /// being removed (ClearParent), for the modal prompt.
    pub parent_label: String,
    pub action: AgentReparentAction,
    /// Mode to restore when the modal closes (drag can start from Terminal or
    /// Navigate).
    pub return_mode: Mode,
}

/// Persistence-neutral projection of a manual-order entry. Panes are keyed by
/// stable (workspace id + public pane number) so they survive the PaneId remap
/// on restore; line-splits carry their id and name directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManualOrderEntryKey {
    Pane {
        workspace_id: String,
        pane_number: usize,
    },
    LineSplit {
        id: u64,
        name: String,
    },
}

/// Flat, client-only manual ordering of agent panes for the agents panel.
///
/// This is TUI presentation state: it never enters the server/runtime protocol.
/// `order` drives the display order, `known` tracks which panes have already
/// been placed (so genuinely new agents get the placement rule), and `seeded`
/// records whether the natural order has been captured at least once.
#[derive(Debug, Clone, Default)]
pub(crate) struct AgentManualOrder {
    pub(crate) order: Vec<ManualEntry>,
    pub(crate) known: std::collections::HashSet<PaneId>,
    pub(crate) seeded: bool,
    /// Monotonic counter handing out [`LineSplitId`]s. Never reused, so ids stay
    /// unique for the lifetime of the session (and across restore).
    pub(crate) next_line_split_id: u64,
}

impl AgentManualOrder {
    /// Insert a new line-split with `name` at flat index `at` (clamped to the
    /// order length) and return its freshly minted id.
    pub(crate) fn new_line_split(&mut self, name: String, at: usize) -> LineSplitId {
        let id = LineSplitId(self.next_line_split_id);
        self.next_line_split_id = self.next_line_split_id.saturating_add(1);
        let at = at.min(self.order.len());
        self.order.insert(at, ManualEntry::LineSplit { id, name });
        id
    }

    /// Rebuild a manual order from persisted stable keys (workspace id + public
    /// pane number) resolved against the restored workspaces. Entries whose
    /// workspace or pane number no longer resolve are dropped. `seeded` is set
    /// because a snapshot was present, so reconcile treats later arrivals as
    /// genuinely new agents rather than reseeding.
    pub(crate) fn from_public_keys(
        keys: &[ManualOrderEntryKey],
        workspaces: &[crate::workspace::Workspace],
    ) -> Self {
        let mut number_to_pane: std::collections::HashMap<(&str, usize), PaneId> =
            std::collections::HashMap::new();
        for ws in workspaces {
            for (pane_id, number) in &ws.public_pane_numbers {
                number_to_pane.insert((ws.id.as_str(), *number), *pane_id);
            }
        }

        let mut order = Vec::new();
        let mut known = std::collections::HashSet::new();
        let mut seen_line_splits = std::collections::HashSet::new();
        let mut max_line_split_id: Option<u64> = None;
        for key in keys {
            match key {
                ManualOrderEntryKey::Pane {
                    workspace_id,
                    pane_number,
                } => {
                    if let Some(pane_id) =
                        number_to_pane.get(&(workspace_id.as_str(), *pane_number))
                    {
                        if known.insert(*pane_id) {
                            order.push(ManualEntry::Pane(*pane_id));
                        }
                    }
                }
                ManualOrderEntryKey::LineSplit { id, name } => {
                    if seen_line_splits.insert(*id) {
                        order.push(ManualEntry::LineSplit {
                            id: LineSplitId(*id),
                            name: name.clone(),
                        });
                        max_line_split_id =
                            Some(max_line_split_id.map_or(*id, |current| current.max(*id)));
                    }
                }
            }
        }

        Self {
            order,
            known,
            seeded: true,
            next_line_split_id: max_line_split_id.map_or(0, |max| max.saturating_add(1)),
        }
    }

    /// Serialize `order` to persistence-neutral keys so the ordering survives the
    /// PaneId remap that happens on restore. Pane entries whose pane cannot be
    /// resolved to a public number are skipped; line-splits are always kept.
    pub(crate) fn to_public_keys(
        &self,
        workspaces: &[crate::workspace::Workspace],
    ) -> Vec<ManualOrderEntryKey> {
        let mut pane_to_key: std::collections::HashMap<PaneId, (String, usize)> =
            std::collections::HashMap::new();
        for ws in workspaces {
            for (pane_id, number) in &ws.public_pane_numbers {
                pane_to_key.insert(*pane_id, (ws.id.clone(), *number));
            }
        }

        self.order
            .iter()
            .filter_map(|entry| match entry {
                ManualEntry::Pane(pane_id) => {
                    pane_to_key.get(pane_id).map(|(workspace_id, pane_number)| {
                        ManualOrderEntryKey::Pane {
                            workspace_id: workspace_id.clone(),
                            pane_number: *pane_number,
                        }
                    })
                }
                ManualEntry::LineSplit { id, name } => Some(ManualOrderEntryKey::LineSplit {
                    id: id.0,
                    name: name.clone(),
                }),
            })
            .collect()
    }
}

/// Stable reference to a single non-agent pane, independent of its position
/// within a workspace. Panes are addressed by their owning workspace id plus
/// their stable public pane number (which survives reorders and the PaneId remap
/// on restore), so a `PaneSectionRef` can be persisted and rebuilt directly
/// without a remap. Client-only presentation state; never enters the
/// server/runtime protocol.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PaneSectionRef {
    pub(crate) workspace_id: String,
    pub(crate) pane_number: usize,
}

/// A single entry in the manual Panes-section ordering. Either a non-agent pane
/// (keyed by its stable [`PaneSectionRef`]) or a user-created named line-split
/// divider, interleaved in one flat vector. Mirrors [`ManualEntry`] for the
/// agents panel; the two lists are independent. Client-only.
#[derive(Debug, Clone)]
pub(crate) enum PaneManualEntry {
    Pane(PaneSectionRef),
    LineSplit { id: LineSplitId, name: String },
}

/// Reference to a single Panes-section entry, used to pick up and move either a
/// pane or a line-split during drag-and-drop. Mirrors [`ManualEntryRef`].
/// Client-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaneManualEntryRef {
    Pane(PaneSectionRef),
    LineSplit(LineSplitId),
}

/// Persistence-neutral projection of a Panes-section entry. Panes carry their
/// stable (workspace id + public pane number); line-splits carry their id and
/// name directly. Mirrors [`ManualOrderEntryKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaneManualEntryKey {
    Pane {
        workspace_id: String,
        pane_number: usize,
    },
    LineSplit {
        id: u64,
        name: String,
    },
}

/// Flat, client-only manual ordering of non-agent panes for the sidebar Panes
/// section.
///
/// This is TUI presentation state: it never enters the server/runtime protocol
/// and never changes the real pane order inside any workspace. `order` drives the
/// display order across all spaces (panes and line-splits interleaved), `known`
/// tracks which panes have already been placed (so genuinely new panes get the
/// placement rule), and `seeded` records whether the natural order has been
/// captured at least once.
#[derive(Debug, Clone, Default)]
pub(crate) struct PaneSectionOrder {
    pub(crate) order: Vec<PaneManualEntry>,
    pub(crate) known: std::collections::HashSet<PaneSectionRef>,
    pub(crate) seeded: bool,
    /// Monotonic counter handing out [`LineSplitId`]s for this section. Never
    /// reused, so ids stay unique for the lifetime of the session (and across
    /// restore). Independent from the agents-panel counter.
    pub(crate) next_line_split_id: u64,
}

impl PaneSectionOrder {
    /// Insert a new line-split with `name` at flat index `at` (clamped to the
    /// order length) and return its freshly minted id.
    pub(crate) fn new_line_split(&mut self, name: String, at: usize) -> LineSplitId {
        let id = LineSplitId(self.next_line_split_id);
        self.next_line_split_id = self.next_line_split_id.saturating_add(1);
        let at = at.min(self.order.len());
        self.order
            .insert(at, PaneManualEntry::LineSplit { id, name });
        id
    }

    /// Rebuild a pane-section order from persisted keys, keeping only pane
    /// entries whose workspace still exists. Line-splits are always retained.
    /// `seeded` is set because a snapshot was present, so reconcile treats later
    /// arrivals as genuinely new panes rather than reseeding.
    pub(crate) fn from_entry_keys(
        keys: &[PaneManualEntryKey],
        workspaces: &[crate::workspace::Workspace],
    ) -> Self {
        let live_ids: std::collections::HashSet<&str> =
            workspaces.iter().map(|ws| ws.id.as_str()).collect();
        let mut order = Vec::new();
        let mut known = std::collections::HashSet::new();
        let mut seen_line_splits = std::collections::HashSet::new();
        let mut max_line_split_id: Option<u64> = None;
        for key in keys {
            match key {
                PaneManualEntryKey::Pane {
                    workspace_id,
                    pane_number,
                } => {
                    let pane_ref = PaneSectionRef {
                        workspace_id: workspace_id.clone(),
                        pane_number: *pane_number,
                    };
                    if live_ids.contains(workspace_id.as_str()) && known.insert(pane_ref.clone()) {
                        order.push(PaneManualEntry::Pane(pane_ref));
                    }
                }
                PaneManualEntryKey::LineSplit { id, name } => {
                    if seen_line_splits.insert(*id) {
                        order.push(PaneManualEntry::LineSplit {
                            id: LineSplitId(*id),
                            name: name.clone(),
                        });
                        max_line_split_id =
                            Some(max_line_split_id.map_or(*id, |current| current.max(*id)));
                    }
                }
            }
        }
        Self {
            order,
            known,
            seeded: true,
            next_line_split_id: max_line_split_id.map_or(0, |max| max.saturating_add(1)),
        }
    }

    /// Snapshot the current order as persistence-neutral keys.
    pub(crate) fn to_entry_keys(&self) -> Vec<PaneManualEntryKey> {
        self.order
            .iter()
            .map(|entry| match entry {
                PaneManualEntry::Pane(pane_ref) => PaneManualEntryKey::Pane {
                    workspace_id: pane_ref.workspace_id.clone(),
                    pane_number: pane_ref.pane_number,
                },
                PaneManualEntry::LineSplit { id, name } => PaneManualEntryKey::LineSplit {
                    id: id.0,
                    name: name.clone(),
                },
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Settings UI state
// ---------------------------------------------------------------------------

/// Which section of the settings panel is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Theme,
    Sound,
    Toast,
    PaneLabels,
    Experiments,
    Integrations,
}

impl SettingsSection {
    pub const ALL: &[Self] = &[
        Self::Theme,
        Self::Sound,
        Self::Toast,
        Self::PaneLabels,
        Self::Integrations,
        Self::Experiments,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Sound => "sound",
            Self::Toast => "toasts",
            Self::PaneLabels => "pane labels",
            Self::Experiments => "experiments",
            Self::Integrations => "integrations",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExperimentSetting {
    PaneHistory,
    SwitchAsciiInputSourceInPrefix,
}

impl ExperimentSetting {
    pub(crate) const ALL: [Self; 2] = [Self::PaneHistory, Self::SwitchAsciiInputSourceInPrefix];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::PaneHistory => "pane screen history",
            Self::SwitchAsciiInputSourceInPrefix => {
                "switch to ascii input source in prefix (macOS)"
            }
        }
    }

    pub(crate) fn enabled(self, state: &AppState) -> bool {
        match self {
            Self::PaneHistory => state.pane_history_persistence_enabled(),
            Self::SwitchAsciiInputSourceInPrefix => {
                state.switch_ascii_input_source_in_prefix_enabled()
            }
        }
    }
}

/// All built-in theme names in display order.
pub const THEME_NAMES: &[&str] = &[
    "catppuccin",
    "catppuccin-latte",
    "terminal",
    "tokyo-night",
    "tokyo-night-day",
    "dracula",
    "nord",
    "gruvbox",
    "gruvbox-light",
    "one-dark",
    "one-light",
    "solarized",
    "solarized-light",
    "kanagawa",
    "kanagawa-lotus",
    "rose-pine",
    "rose-pine-dawn",
    "vesper",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuListState {
    pub highlighted: usize,
}

impl MenuListState {
    pub fn new(highlighted: usize) -> Self {
        Self { highlighted }
    }

    pub fn move_prev(&mut self) {
        self.highlighted = self.highlighted.saturating_sub(1);
    }

    pub fn move_next(&mut self, item_count: usize) {
        if item_count > 0 {
            self.highlighted = (self.highlighted + 1).min(item_count - 1);
        }
    }

    pub fn hover(&mut self, idx: Option<usize>) {
        if let Some(idx) = idx {
            self.highlighted = idx;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionListState {
    pub selected: usize,
}

impl SelectionListState {
    pub fn new(selected: usize) -> Self {
        Self { selected }
    }

    pub fn move_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_next(&mut self, item_count: usize) {
        if item_count > 0 {
            self.selected = (self.selected + 1).min(item_count - 1);
        }
    }

    pub fn select(&mut self, idx: usize) {
        self.selected = idx;
    }
}

#[derive(Debug, Clone)]
pub struct ThemeRuntimeConfig {
    pub manual_name: String,
    pub dark_name: String,
    pub light_name: String,
    pub auto_switch: bool,
    pub custom: Option<crate::config::CustomThemeColors>,
    pub legacy_accent: Option<String>,
}

pub struct SettingsState {
    /// Which section tab is active.
    pub section: SettingsSection,
    /// Selected item index within the current section.
    pub list: SelectionListState,
    /// The palette before opening settings (for cancel/restore).
    pub original_palette: Option<Palette>,
    /// The theme name before opening settings.
    pub original_theme: Option<String>,
}

pub(crate) enum DragTarget {
    WorkspaceReorder {
        source_ws_idx: usize,
        insert_idx: Option<usize>,
    },
    TabReorder {
        ws_idx: usize,
        source_tab_idx: usize,
        insert_idx: Option<usize>,
    },
    /// Reorder of a manual-order entry (agent pane or line-split) within the flat
    /// manual agent ordering. Cross-workspace moves are allowed; `insert_idx` is
    /// a flat index into the manual order.
    AgentReorder {
        source: ManualEntryRef,
        insert_idx: Option<usize>,
    },
    /// Reorder of a Panes-section entry (non-agent pane or line-split) within the
    /// flat, client-only Panes-section ordering. Cross-workspace moves are
    /// allowed; this only changes the sidebar's visual order, never the real tab
    /// order inside any workspace. `insert_idx` is a flat index into the
    /// pane-section order.
    PaneSectionReorder {
        source: PaneManualEntryRef,
        insert_idx: Option<usize>,
    },
    WorkspaceListScrollbar {
        grab_row_offset: u16,
    },
    AgentPanelScrollbar {
        grab_row_offset: u16,
    },
    PaneSectionScrollbar {
        grab_row_offset: u16,
    },
    PaneSplit {
        path: Vec<bool>,
        direction: Direction,
        area: Rect,
        grab_offset: u16,
    },
    PaneScrollbar {
        pane_id: crate::layout::PaneId,
        grab_row_offset: u16,
    },
    ReleaseNotesScrollbar {
        grab_row_offset: u16,
    },
    ProductAnnouncementScrollbar {
        grab_row_offset: u16,
    },
    KeybindHelpScrollbar {
        grab_row_offset: u16,
    },
    SidebarDivider,
    /// Drag of one of the two sidebar section dividers. `index` selects which
    /// divider: 0 = Spaces/Panes, 1 = Panes/Agents.
    SidebarSectionDivider {
        index: usize,
    },
}

/// Active mouse drag on a split border or sidebar divider.
pub(crate) struct DragState {
    pub target: DragTarget,
}

pub(crate) struct WorkspacePressState {
    pub ws_idx: usize,
    pub start_col: u16,
    pub start_row: u16,
}

pub(crate) struct TabPressState {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub start_col: u16,
    pub start_row: u16,
}

pub(crate) struct AgentPressState {
    pub entry: ManualEntryRef,
    pub start_col: u16,
    pub start_row: u16,
}

pub(crate) struct PaneSectionPressState {
    pub entry: PaneManualEntryRef,
    pub start_col: u16,
    pub start_row: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuKind {
    Workspace {
        ws_idx: usize,
    },
    GitWorkspace {
        ws_idx: usize,
        is_linked_worktree: bool,
        has_worktree_children: bool,
        collapsed: bool,
    },
    Tab {
        ws_idx: usize,
        tab_idx: usize,
    },
    Pane {
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
        source_pane_id: Option<PaneId>,
        has_manual_label: bool,
    },
    /// Right-click menu for a named line-split divider row. `section` selects
    /// which independent order (agents panel vs Panes section) the split lives in.
    LineSplit {
        section: LineSplitSection,
        id: LineSplitId,
    },
}

/// Right-click context menu state.
pub struct ContextMenuState {
    pub kind: ContextMenuKind,
    pub x: u16,
    pub y: u16,
    pub list: MenuListState,
}

impl ContextMenuState {
    pub fn items(&self) -> &'static [&'static str] {
        match self.kind {
            ContextMenuKind::Workspace { .. } => &["Rename", "Close"],
            ContextMenuKind::GitWorkspace {
                is_linked_worktree: false,
                has_worktree_children: false,
                ..
            } => &["Rename", "Close", "New worktree", "Open worktree..."],
            ContextMenuKind::GitWorkspace {
                is_linked_worktree: true,
                ..
            } => &["Rename", "Close", "Delete worktree checkout..."],
            ContextMenuKind::GitWorkspace {
                is_linked_worktree: false,
                has_worktree_children: true,
                collapsed: true,
                ..
            } => &[
                "Rename",
                "Close group",
                "New worktree",
                "Open worktree...",
                "Expand",
            ],
            ContextMenuKind::GitWorkspace {
                is_linked_worktree: false,
                has_worktree_children: true,
                collapsed: false,
                ..
            } => &[
                "Rename",
                "Close group",
                "New worktree",
                "Open worktree...",
                "Collapse",
            ],
            ContextMenuKind::Tab { .. } => &["New tab", "Rename", "Close"],
            ContextMenuKind::Pane {
                has_manual_label: true,
                source_pane_id: Some(_),
                ..
            } => &[
                "Rename pane",
                "Clear pane name",
                "Swap with focused pane",
                "Split right",
                "Split down",
                "Zoom",
                "Close pane",
            ],
            ContextMenuKind::Pane {
                has_manual_label: false,
                source_pane_id: Some(_),
                ..
            } => &[
                "Rename pane",
                "Swap with focused pane",
                "Split right",
                "Split down",
                "Zoom",
                "Close pane",
            ],
            ContextMenuKind::Pane {
                has_manual_label: true,
                source_pane_id: None,
                ..
            } => &[
                "Rename pane",
                "Clear pane name",
                "Split right",
                "Split down",
                "Zoom",
                "Close pane",
            ],
            ContextMenuKind::Pane {
                has_manual_label: false,
                source_pane_id: None,
                ..
            } => &[
                "Rename pane",
                "Split right",
                "Split down",
                "Zoom",
                "Close pane",
            ],
            ContextMenuKind::LineSplit { .. } => &["Rename", "Delete"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    NeedsAttention,
    Finished,
    UpdateInstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastTarget {
    pub workspace_id: String,
    pub pane_id: PaneId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastNotification {
    pub kind: ToastKind,
    pub title: String,
    pub context: String,
    pub position: Option<crate::config::ToastHerdrPosition>,
    pub target: Option<ToastTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAgentNotification {
    pub pane_id: PaneId,
    pub workspace_id: String,
    pub agent_label: String,
    pub known_agent: Option<crate::detect::Agent>,
    pub kind: ToastKind,
    pub state: AgentState,
    pub deadline: std::time::Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNotificationDelivery {
    pub pane_id: PaneId,
    pub workspace_id: String,
    pub agent_label: String,
    pub known_agent: Option<crate::detect::Agent>,
    pub kind: ToastKind,
    pub toast: Option<ToastNotification>,
    pub client_notification: Option<ToastNotification>,
    pub sound: Option<crate::sound::Sound>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFeedback {
    pub message: String,
}

pub struct ReleaseNotesState {
    pub version: String,
    pub body: String,
    pub scroll: u16,
    pub preview: bool,
}

pub struct ProductAnnouncementState {
    pub version: String,
    pub id: String,
    pub title: String,
    pub body: String,
    pub scroll: u16,
    pub preview: bool,
}

pub struct KeybindHelpState {
    pub scroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarWidthSource {
    ConfigDefault,
    Persisted,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneFocusTarget {
    pub workspace_id: String,
    pub pane_id: PaneId,
}

/// A workspace captured on close, with the display index it occupied, so undo
/// can reopen it in place.
pub(crate) struct ClosedWorkspaceEntry {
    pub index: usize,
    pub snapshot: Box<crate::persist::WorkspaceSnapshot>,
}

/// A recently closed tab or workspace group, captured for undo-close. Reopening
/// spawns fresh shells from the snapshot; scrollback and running processes are
/// not revived.
pub(crate) enum ClosedEntry {
    /// One or more workspaces closed together. A worktree group closes as a set,
    /// so it reopens as a set.
    Workspaces(Vec<ClosedWorkspaceEntry>),
    /// A tab closed while its workspace stayed open.
    Tab {
        /// Stable id of the workspace the tab belongs to.
        workspace_id: String,
        /// Tab display index at close time; reopen clamps to bounds.
        index: usize,
        snapshot: Box<crate::persist::TabSnapshot>,
    },
}

/// Maximum number of closed tabs/workspaces retained for undo. Oldest entries
/// are dropped past this bound.
pub(crate) const MAX_CLOSED_ENTRIES: usize = 25;

/// Which of the three stacked sidebar bands (Spaces / Panes / Agents) are
/// collapsed to a single header row. Client-only TUI presentation state.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SidebarSectionCollapse {
    pub spaces: bool,
    pub panes: bool,
    pub agents: bool,
}

/// All application state — pure data, no channels or async runtime.
/// Testable without PTYs or a tokio runtime.
pub struct AppState {
    pub terminals:
        std::collections::HashMap<crate::terminal::TerminalId, crate::terminal::TerminalState>,
    /// Terminal ids whose size is currently owned by a direct attach client.
    pub direct_attach_resize_locks: std::collections::HashSet<crate::terminal::TerminalId>,
    pub(crate) pane_id_aliases: std::collections::HashMap<u32, PaneId>,
    pub(crate) public_pane_id_aliases: std::collections::HashMap<String, PaneId>,
    pub workspaces: Vec<Workspace>,
    pub active: Option<usize>,
    pub(crate) previous_pane_focus: Option<PaneFocusTarget>,
    /// Last pane focused within the sidebar agents section this session. Lets an
    /// agent-nav key jump back to the last selected agent when focus is currently
    /// outside the agents section. Client-only TUI presentation state; ephemeral,
    /// not persisted and not part of the wire protocol.
    pub(crate) last_agent_focus: Option<PaneId>,
    /// Last pane focused within the sidebar Panes section this session. Mirrors
    /// [`AppState::last_agent_focus`] for the Panes section. Client-only TUI
    /// presentation state; ephemeral, not persisted and not part of the wire
    /// protocol.
    pub(crate) last_pane_section_focus: Option<PaneId>,
    pub selected: usize,
    pub mode: Mode,
    pub should_quit: bool,
    /// In monolithic --no-session mode, detach exits the app because there is no server to detach from.
    pub detach_exits: bool,
    /// Set when the current client should detach from the persistent session.
    /// The server's event loop checks this and handles client detach.
    pub detach_requested: bool,
    pub request_new_workspace: bool,
    pub request_new_tab: bool,
    /// Set when input requested reopening the most recently closed tab or
    /// workspace. Drained by the outer App/event loop, which owns the runtime
    /// context needed to respawn shells.
    pub request_undo_close: bool,
    /// LIFO stack of recently closed tabs/workspaces available to reopen.
    /// Runtime convenience state; not persisted across sessions.
    pub(crate) closed_entries: Vec<ClosedEntry>,
    pub request_new_linked_worktree: Option<usize>,
    pub request_open_existing_worktree: Option<usize>,
    pub request_new_workspace_cwd: Option<std::path::PathBuf>,
    pub request_remove_linked_worktree: Option<usize>,
    pub request_submit_worktree_create: bool,
    pub request_submit_worktree_open: bool,
    pub request_submit_worktree_remove: bool,
    pub request_reload_config: bool,
    /// Set when the headless server should ask attached clients to reload
    /// their client-local sound config from disk.
    pub request_client_config_reload: bool,
    /// Set when UI interaction requested a clipboard write that must be
    /// handled by the outer App/event loop instead of directly from AppState.
    pub request_clipboard_write: Option<Vec<u8>>,
    pub creating_new_tab: bool,
    pub requested_new_tab_name: Option<String>,
    pub rename_pane_target: Option<PaneId>,
    /// Target line-split for the rename modal (client-only; mirrors
    /// `rename_pane_target`). Carries the owning section so the commit writes into
    /// the right order. Set only while `mode == Mode::RenameLineSplit`.
    pub(crate) rename_line_split_target: Option<(LineSplitSection, LineSplitId)>,
    /// Target `(ws_idx, tab_idx)` for the tab rename modal (client-only; mirrors
    /// `rename_pane_target`). When set, the RenameTab commit renames this tab
    /// instead of the active tab. Set only while `mode == Mode::RenameTab`.
    pub(crate) rename_tab_target: Option<(usize, usize)>,
    pub worktree_create: Option<WorktreeCreateState>,
    pub worktree_open: Option<WorktreeOpenState>,
    pub worktree_remove: Option<WorktreeRemoveState>,
    pub worktree_directory: std::path::PathBuf,
    pub collapsed_space_keys: std::collections::HashSet<String>,
    /// Public pane ids (e.g. `"w1:p2"`) of agent-tree parents the user has
    /// collapsed in the agents panel. Client-only TUI presentation state,
    /// persisted like [`AppState::collapsed_space_keys`].
    pub collapsed_agent_keys: std::collections::HashSet<String>,
    pub request_complete_onboarding: bool,
    pub name_input: String,
    pub name_input_replace_on_type: bool,
    /// Caret position within `name_input`, measured as a char index in
    /// `0..=name_input.chars().count()`. TUI/client presentation state only.
    pub name_input_cursor: usize,
    pub release_notes: Option<ReleaseNotesState>,
    pub product_announcement: Option<ProductAnnouncementState>,
    pub keybind_help: KeybindHelpState,
    pub navigator: NavigatorState,
    pub copy_mode: Option<CopyModeState>,
    pub(crate) copy_search: Option<CopySearchState>,
    pub workspace_scroll: usize,
    pub agent_panel_scroll: usize,
    /// Scroll offset (in flat order index) for the sidebar Panes section.
    pub pane_section_scroll: usize,
    pub tab_scroll: usize,
    pub tab_scroll_follow_active: bool,
    pub mobile_switcher_scroll: usize,
    // View geometry (computed before render, consumed by render + mouse)
    pub view: ViewState,
    pub(crate) drag: Option<DragState>,
    pub(crate) workspace_press: Option<WorkspacePressState>,
    pub(crate) tab_press: Option<TabPressState>,
    pub(crate) agent_press: Option<AgentPressState>,
    pub(crate) pane_section_press: Option<PaneSectionPressState>,
    pub selection: Option<Selection>,
    pub selection_autoscroll: Option<SelectionAutoscroll>,
    pub context_menu: Option<ContextMenuState>,
    // Notifications
    pub update_available: Option<String>,
    pub update_install_command: String,
    pub latest_release_notes_available: bool,
    pub update_dismissed: bool,
    pub config_diagnostic: Option<String>,
    pub toast: Option<ToastNotification>,
    pub pending_agent_notifications: std::collections::HashMap<PaneId, PendingAgentNotification>,
    pub copy_feedback: Option<CopyFeedback>,
    /// Last reported focus state for the outer terminal hosting herdr.
    /// None means unsupported or not yet reported, which preserves active-pane suppression.
    pub outer_terminal_focus: Option<bool>,
    // Config
    pub prefix_code: KeyCode,
    pub prefix_mods: KeyModifiers,
    pub default_sidebar_width: u16,
    pub sidebar_width: u16,
    pub sidebar_min_width: u16,
    pub sidebar_max_width: u16,
    pub mobile_width_threshold: u16,
    pub sidebar_width_source: SidebarWidthSource,
    pub sidebar_width_auto: bool,
    pub sidebar_collapsed: bool,
    pub sidebar_collapsed_mode: crate::config::SidebarCollapsedModeConfig,
    /// Ratio of total sidebar height allocated to the Spaces section (the first
    /// of three stacked bands: Spaces / Tabs / Agents).
    pub sidebar_section_split: f32,
    /// Ratio of the remaining sidebar height (below the Spaces band) allocated to
    /// the Panes section; the Agents section takes the rest.
    pub sidebar_pane_section_split: f32,
    /// Whether the Spaces band is collapsed to a single header row. Client-only
    /// TUI presentation state (mirrors [`AppState::sidebar_collapsed`], which
    /// collapses the whole sidebar); ephemeral, not persisted.
    pub(crate) spaces_section_collapsed: bool,
    /// Whether the Panes band is collapsed to a single header row. Client-only
    /// TUI presentation state; ephemeral, not persisted.
    pub(crate) pane_section_collapsed: bool,
    /// Whether the Agents band is collapsed to a single header row. Client-only
    /// TUI presentation state; ephemeral, not persisted.
    pub(crate) agents_section_collapsed: bool,
    pub agent_panel_sort: AgentPanelSort,
    /// Flat client-only manual ordering of agent panes (TUI presentation state).
    pub(crate) agent_manual_order: AgentManualOrder,
    /// Flat client-only manual ordering of non-agent panes for the Panes section
    /// (TUI presentation state).
    pub(crate) pane_section_order: PaneSectionOrder,
    /// A drag-to-reparent operation awaiting confirmation. Set only while
    /// `mode == Mode::ConfirmAgentReparent`.
    pub(crate) pending_agent_reparent: Option<PendingAgentReparent>,
    pub next_agent_state_change_seq: u64,
    /// Capture mouse input for Herdr's own mouse UI. When false, Herdr only
    /// captures mouse while the focused pane app requests mouse reporting.
    pub mouse_capture: bool,
    pub right_click_passthrough_modifiers: Option<KeyModifiers>,
    pub right_click_passthrough: Option<RightClickPassthroughGesture>,
    pub redraw_on_focus_gained: bool,
    pub mouse_scroll_lines: usize,
    pub confirm_close: bool,
    pub prompt_new_tab_name: bool,
    pub pane_borders: bool,
    pub pane_gaps: bool,
    pub show_agent_labels_on_pane_borders: bool,
    pub pane_history_persistence: bool,
    /// Expose the focused pane's cursor anchor to the outer terminal even when
    /// the pane requested `?25l`. See `[experimental] reveal_hidden_cursor_for_cjk_ime`.
    pub reveal_hidden_cursor_for_cjk_ime: bool,
    /// Restrict cursor reveal to focused panes whose detected agent matches
    /// one of these. When false, apply to any focused pane.
    pub cjk_ime_agent_filter_configured: bool,
    pub cjk_ime_agents: Vec<crate::detect::Agent>,
    /// DECSCUSR shape parameter (1–6) for the IME anchor cursor.
    pub cjk_ime_cursor_shape: u8,
    /// While prefix mode is active, switch the macOS host input source to an
    /// ASCII-capable layout so prefix commands register as ASCII even when a
    /// CJK IME is active. macOS only; a no-op elsewhere. See
    /// `[experimental] switch_ascii_input_source_in_prefix`.
    pub switch_ascii_input_source_in_prefix: bool,
    pub kitty_graphics_enabled: bool,
    pub default_shell: String,
    pub shell_mode: crate::config::ShellModeConfig,
    pub new_terminal_cwd: NewTerminalCwdConfig,
    pub pane_scrollback_limit_bytes: usize,
    #[allow(dead_code)] // kept for backward compat; palette.accent is the source of truth
    pub accent: Color,
    pub sound: SoundConfig,
    pub local_sound_playback: bool,
    pub toast_config: ToastConfig,
    pub keybinds: Keybinds,
    /// Frame counter for spinner animations (wraps around).
    pub spinner_tick: u32,
    /// UI color palette — all sidebar/UI colors centralized for theming.
    pub palette: Palette,
    /// Currently applied theme name (for settings UI).
    pub theme_name: String,
    /// Runtime theme configuration used to resolve manual and auto-switch palettes.
    pub theme_runtime: ThemeRuntimeConfig,
    /// Last known foreground host terminal appearance.
    pub host_terminal_appearance: Option<HostAppearance>,
    /// True when the foreground host explicitly reported appearance via Mode 2031.
    pub host_terminal_appearance_explicit: bool,
    /// Settings panel state.
    pub settings: SettingsState,
    /// Cached integration recommendations for onboarding/settings UI.
    pub integration_recommendations: Vec<crate::integration::IntegrationRecommendation>,
    /// Cached detection manifest source/version summaries for runtime/API status.
    pub agent_manifest_summaries: Vec<crate::detect::manifest::AgentManifestSummary>,
    /// Cached remote detection manifest update diagnostics for runtime/API status.
    pub agent_manifest_update_status: crate::detect::manifest_update::ManifestUpdateStatus,
    /// Result messages from the latest integration install action.
    pub integration_install_messages: Vec<String>,
    /// Installed or linked plugins known to this running Herdr instance.
    pub(crate) installed_plugins: InstalledPluginRegistry,
    /// Pane ids opened through the plugin pane API.
    pub(crate) plugin_panes: std::collections::HashMap<PaneId, PluginPaneRecord>,
    /// Recent plugin action/event command executions.
    pub(crate) plugin_command_logs: Vec<crate::api::schema::PluginCommandLogInfo>,
    pub(crate) next_plugin_command_log_id: u64,
    pub(crate) plugin_commands_in_flight: usize,
    /// Highlight state for the bottom-right global launcher menu.
    pub global_menu: MenuListState,
    /// Resolved host terminal default colors for theming embedded panes.
    pub host_terminal_theme: TerminalTheme,
    /// Set when a persisted session snapshot would change.
    pub session_dirty: bool,
    /// Terminal runtimes that should be shut down by the app/runtime layer
    /// after state has detached their terminal metadata.
    pub(crate) terminal_runtime_shutdowns: Vec<crate::terminal::TerminalId>,
}

impl AppState {
    pub(crate) fn mark_session_dirty(&mut self) {
        self.session_dirty = true;
    }

    /// Which stacked sidebar bands are currently collapsed to a header row.
    pub(crate) fn sidebar_section_collapse(&self) -> SidebarSectionCollapse {
        SidebarSectionCollapse {
            spaces: self.spaces_section_collapsed,
            panes: self.pane_section_collapsed,
            agents: self.agents_section_collapsed,
        }
    }

    /// Resolve a stable [`crate::pane::PaneParentRef`] to the live
    /// `(ws_idx, PaneId)` of the parent pane, if it still exists. Returns `None`
    /// when the referenced workspace or pane number is gone (e.g. the parent was
    /// closed), which is how a child becomes a root in the agents-panel tree.
    pub(crate) fn resolve_pane_parent(
        &self,
        parent: &crate::pane::PaneParentRef,
    ) -> Option<(usize, PaneId)> {
        let ws_idx = self
            .workspaces
            .iter()
            .position(|ws| ws.id == parent.workspace_id)?;
        let pane_id = self.workspaces[ws_idx]
            .public_pane_numbers
            .iter()
            .find_map(|(pane_id, number)| (*number == parent.pane_number).then_some(*pane_id))?;
        Some((ws_idx, pane_id))
    }

    /// Set the rename/name input text and place the caret at the end. Keeps
    /// `name_input` and `name_input_cursor` consistent for all callers.
    pub(crate) fn set_name_input(&mut self, value: impl Into<String>) {
        self.name_input = value.into();
        self.name_input_cursor = self.name_input.chars().count();
    }

    /// Number of caret positions in `name_input` (char count).
    pub(crate) fn name_input_len_chars(&self) -> usize {
        self.name_input.chars().count()
    }

    pub(crate) fn remove_alias_shadowed_by_new_pane(&mut self, pane_id: PaneId) {
        self.pane_id_aliases.remove(&pane_id.raw());
    }

    pub fn sound_enabled(&self) -> bool {
        self.sound.enabled
    }

    pub fn toast_delivery(&self) -> ToastDelivery {
        self.toast_config.delivery
    }

    pub fn agent_border_labels_enabled(&self) -> bool {
        self.show_agent_labels_on_pane_borders
    }

    pub fn pane_history_persistence_enabled(&self) -> bool {
        self.pane_history_persistence
    }

    pub fn switch_ascii_input_source_in_prefix_enabled(&self) -> bool {
        self.switch_ascii_input_source_in_prefix
    }

    pub(crate) fn pane_exposes_host_cursor(
        &self,
        _ws_idx: usize,
        _pane_id: crate::layout::PaneId,
    ) -> bool {
        true
    }

    pub(crate) fn integration_updates_available(&self) -> bool {
        self.integration_recommendations
            .iter()
            .any(|item| item.state == crate::integration::IntegrationStatusKind::Outdated)
    }

    pub(crate) fn refresh_agent_manifest_summaries(&mut self) {
        self.agent_manifest_summaries = crate::detect::manifest::manifest_summaries();
    }

    pub(crate) fn global_menu_attention_badge_visible(&self) -> bool {
        self.update_available.is_some() || self.integration_updates_available()
    }

    pub(crate) fn global_menu_item_has_badge(&self, item: &str) -> bool {
        (item == "update ready" && self.update_available.is_some())
            || (item == "settings" && self.integration_updates_available())
    }

    pub(crate) fn settings_section_has_badge(&self, section: SettingsSection) -> bool {
        section == SettingsSection::Integrations && self.integration_updates_available()
    }

    pub(crate) fn focused_pane_requests_mouse_capture_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> bool {
        self.mode == Mode::Terminal
            && self
                .active
                .and_then(|idx| self.focused_runtime_in_workspace(terminal_runtimes, idx))
                .and_then(crate::terminal::TerminalRuntime::input_state)
                .is_some_and(crate::pane::InputState::mouse_reporting_enabled)
    }

    pub(crate) fn should_capture_host_mouse_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> bool {
        self.mouse_capture || self.focused_pane_requests_mouse_capture_from(terminal_runtimes)
    }

    pub fn is_prefix_key(&self, key: crate::input::TerminalKey) -> bool {
        crate::config::terminal_key_matches_combo(key, (self.prefix_code, self.prefix_mods))
    }

    pub fn estimate_pane_size(&self) -> (u16, u16) {
        if let Some(info) = self.view.pane_infos.first() {
            (info.rect.height, info.rect.width)
        } else {
            (24, 80)
        }
    }

    /// Returns true when the given (workspace, tab, pane) refers to the
    /// currently focused pane in the active workspace's active tab.
    pub(crate) fn runtime_for_pane_in_workspace<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        #[cfg(test)]
        if let Some(runtime) = self.workspaces.get(ws_idx)?.test_runtimes.get(&pane_id) {
            return Some(runtime);
        }
        #[cfg(test)]
        if let Some(runtime) = self
            .workspaces
            .get(ws_idx)?
            .tabs
            .iter()
            .find_map(|tab| tab.runtimes.get(&pane_id))
        {
            return Some(runtime);
        }
        let terminal_id = self.workspaces.get(ws_idx)?.terminal_id(pane_id)?;
        terminal_runtimes.get(terminal_id)
    }

    #[cfg(test)]
    pub(crate) fn runtime_for_pane<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        self.workspaces.iter().find_map(|ws| {
            #[cfg(test)]
            if let Some(runtime) = ws.test_runtimes.get(&pane_id) {
                return Some(runtime);
            }
            #[cfg(test)]
            if let Some(runtime) = ws.tabs.iter().find_map(|tab| tab.runtimes.get(&pane_id)) {
                return Some(runtime);
            }
            let terminal_id = ws.terminal_id(pane_id)?;
            terminal_runtimes.get(terminal_id)
        })
    }

    pub(crate) fn focused_runtime_in_workspace<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        ws_idx: usize,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        let ws = self.workspaces.get(ws_idx)?;
        let pane_id = ws.focused_pane_id()?;
        self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
    }

    pub fn is_active_pane(
        &self,
        ws_idx: usize,
        tab_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> bool {
        let Some(active_ws_idx) = self.active else {
            return false;
        };
        if ws_idx != active_ws_idx {
            return false;
        }
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return false;
        };
        if tab_idx != ws.active_tab_index() {
            return false;
        }
        ws.active_tab().map(|tab| tab.layout.focused()) == Some(pane_id)
    }
}

#[cfg(test)]
pub fn key_matches(
    key: &crossterm::event::KeyEvent,
    expected_code: KeyCode,
    expected_mods: KeyModifiers,
) -> bool {
    crate::config::terminal_key_matches_combo(
        crate::input::TerminalKey::from(*key),
        (expected_code, expected_mods),
    )
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
impl AppState {
    /// Create an AppState for testing — no channels, no PTYs.
    pub fn test_new() -> Self {
        Self {
            terminals: std::collections::HashMap::new(),
            direct_attach_resize_locks: std::collections::HashSet::new(),
            pane_id_aliases: std::collections::HashMap::new(),
            public_pane_id_aliases: std::collections::HashMap::new(),
            workspaces: Vec::new(),
            active: None,
            previous_pane_focus: None,
            last_agent_focus: None,
            last_pane_section_focus: None,
            selected: 0,
            mode: Mode::Navigate,
            should_quit: false,
            detach_exits: false,
            detach_requested: false,
            request_new_workspace: false,
            request_new_tab: false,
            request_undo_close: false,
            closed_entries: Vec::new(),
            request_new_linked_worktree: None,
            request_open_existing_worktree: None,
            request_new_workspace_cwd: None,
            request_remove_linked_worktree: None,
            request_submit_worktree_create: false,
            request_submit_worktree_open: false,
            request_submit_worktree_remove: false,
            request_reload_config: false,
            request_client_config_reload: false,
            request_clipboard_write: None,
            creating_new_tab: false,
            requested_new_tab_name: None,
            rename_pane_target: None,
            rename_line_split_target: None,
            rename_tab_target: None,
            worktree_create: None,
            worktree_open: None,
            worktree_remove: None,
            worktree_directory: std::path::PathBuf::from("/tmp/herdr-worktrees"),
            collapsed_space_keys: std::collections::HashSet::new(),
            collapsed_agent_keys: std::collections::HashSet::new(),
            request_complete_onboarding: false,
            name_input: String::new(),
            name_input_replace_on_type: false,
            name_input_cursor: 0,
            release_notes: None,
            product_announcement: None,
            keybind_help: KeybindHelpState { scroll: 0 },
            navigator: NavigatorState::default(),
            copy_mode: None,
            copy_search: None,
            workspace_scroll: 0,
            agent_panel_scroll: 0,
            pane_section_scroll: 0,
            tab_scroll: 0,
            tab_scroll_follow_active: true,
            mobile_switcher_scroll: 0,
            view: ViewState {
                layout: ViewLayout::Desktop,
                sidebar_rect: Rect::default(),
                workspace_card_areas: Vec::new(),
                pane_section_row_areas: Vec::new(),
                tab_bar_rect: Rect::default(),
                tab_hit_areas: Vec::new(),
                tab_scroll_left_hit_area: Rect::default(),
                tab_scroll_right_hit_area: Rect::default(),
                new_tab_hit_area: Rect::default(),
                terminal_area: Rect::default(),
                mobile_header_rect: Rect::default(),
                mobile_menu_hit_area: Rect::default(),
                toast_hit_area: Rect::default(),
                pane_infos: Vec::new(),
                split_borders: Vec::new(),
            },
            drag: None,
            workspace_press: None,
            tab_press: None,
            agent_press: None,
            pane_section_press: None,
            selection: None,
            selection_autoscroll: None,
            context_menu: None,
            update_available: None,
            update_install_command: "herdr update".into(),
            latest_release_notes_available: false,
            update_dismissed: false,
            config_diagnostic: None,
            toast: None,
            pending_agent_notifications: std::collections::HashMap::new(),
            copy_feedback: None,
            outer_terminal_focus: None,
            prefix_code: KeyCode::Char('b'),
            prefix_mods: KeyModifiers::CONTROL,
            default_sidebar_width: 26,
            sidebar_width: 26,
            sidebar_min_width: 18,
            sidebar_max_width: 36,
            mobile_width_threshold: crate::config::DEFAULT_MOBILE_WIDTH_THRESHOLD,
            sidebar_width_source: SidebarWidthSource::ConfigDefault,
            sidebar_width_auto: false,
            sidebar_collapsed: false,
            sidebar_collapsed_mode: crate::config::SidebarCollapsedModeConfig::Compact,
            sidebar_section_split: 0.5,
            sidebar_pane_section_split: 0.5,
            spaces_section_collapsed: false,
            pane_section_collapsed: false,
            agents_section_collapsed: false,
            agent_panel_sort: AgentPanelSort::Spaces,
            agent_manual_order: AgentManualOrder::default(),
            pane_section_order: PaneSectionOrder::default(),
            pending_agent_reparent: None,
            next_agent_state_change_seq: 0,
            mouse_capture: true,
            right_click_passthrough_modifiers: None,
            right_click_passthrough: None,
            redraw_on_focus_gained: true,
            mouse_scroll_lines: crate::config::DEFAULT_MOUSE_SCROLL_LINES,
            confirm_close: true,
            prompt_new_tab_name: true,
            pane_borders: true,
            pane_gaps: false,
            show_agent_labels_on_pane_borders: false,
            pane_history_persistence: false,
            reveal_hidden_cursor_for_cjk_ime: false,
            cjk_ime_agent_filter_configured: false,
            cjk_ime_agents: Vec::new(),
            cjk_ime_cursor_shape: 2, // steady_block
            switch_ascii_input_source_in_prefix: false,
            kitty_graphics_enabled: false,
            default_shell: String::new(),
            shell_mode: crate::config::ShellModeConfig::Auto,
            new_terminal_cwd: NewTerminalCwdConfig::Follow,
            pane_scrollback_limit_bytes: crate::config::DEFAULT_SCROLLBACK_LIMIT_BYTES,
            accent: Color::Cyan,
            sound: SoundConfig {
                enabled: false,
                ..SoundConfig::default()
            },
            local_sound_playback: false,
            toast_config: ToastConfig::default(),
            keybinds: Keybinds::default(),
            spinner_tick: 0,
            palette: Palette::catppuccin(),
            theme_name: "catppuccin".to_string(),
            theme_runtime: ThemeRuntimeConfig {
                manual_name: "catppuccin".to_string(),
                dark_name: "catppuccin".to_string(),
                light_name: "catppuccin-latte".to_string(),
                auto_switch: false,
                custom: None,
                legacy_accent: None,
            },
            host_terminal_appearance: None,
            host_terminal_appearance_explicit: false,
            settings: SettingsState {
                section: SettingsSection::Theme,
                list: SelectionListState::new(0),
                original_palette: None,
                original_theme: None,
            },
            integration_recommendations: Vec::new(),
            agent_manifest_summaries: Vec::new(),
            agent_manifest_update_status:
                crate::detect::manifest_update::ManifestUpdateStatus::default(),
            integration_install_messages: Vec::new(),
            installed_plugins: std::collections::HashMap::new(),
            plugin_panes: std::collections::HashMap::new(),
            plugin_command_logs: Vec::new(),
            next_plugin_command_log_id: 1,
            plugin_commands_in_flight: 0,
            global_menu: MenuListState::new(0),
            host_terminal_theme: TerminalTheme::default(),
            session_dirty: false,
            terminal_runtime_shutdowns: Vec::new(),
        }
    }

    /// Populate missing `TerminalState` entries for every pane so tests that
    /// read or write terminal metadata don't need to manually create them.
    pub fn ensure_test_terminals(&mut self) {
        use crate::terminal::TerminalState;
        for ws in &self.workspaces {
            for tab in &ws.tabs {
                for pane in tab.panes.values() {
                    if !self.terminals.contains_key(&pane.attached_terminal_id) {
                        let cwd = ws.identity_cwd.clone();
                        self.terminals.insert(
                            pane.attached_terminal_id.clone(),
                            TerminalState::new(pane.attached_terminal_id.clone(), cwd),
                        );
                    }
                }
            }
        }
    }

    pub fn test_with_adversarial_identity_state() -> Self {
        let mut state = Self::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_adversarial_identity_state()];
        state.active = Some(0);
        state.selected = 0;
        state.ensure_test_terminals();
        state
    }

    pub fn assert_invariants_for_test(&self) {
        assert!(
            self.pending_agent_reparent.is_none() || self.mode == Mode::ConfirmAgentReparent,
            "pending agent reparent must only be set while the confirm modal is open"
        );

        if self.workspaces.is_empty() {
            assert!(
                self.active.is_none(),
                "empty app state must not have active workspace {:?}",
                self.active
            );
            assert_eq!(
                self.selected, 0,
                "empty app state should keep selected workspace at 0"
            );
            assert!(
                self.pane_id_aliases.is_empty(),
                "empty app state must not keep raw pane aliases"
            );
            assert!(
                self.public_pane_id_aliases.is_empty(),
                "empty app state must not keep public pane aliases"
            );
            assert!(
                self.previous_pane_focus.is_none(),
                "empty app state must not keep previous pane focus"
            );
            assert!(
                self.plugin_panes.is_empty(),
                "empty app state must not keep plugin pane records"
            );
            assert!(
                self.pending_agent_notifications.is_empty(),
                "empty app state must not keep pending agent notifications"
            );
            assert!(
                self.copy_mode.is_none(),
                "empty app state must not keep copy mode"
            );
            assert!(
                self.rename_pane_target.is_none(),
                "empty app state must not keep rename pane target"
            );
            assert!(
                self.rename_line_split_target.is_none(),
                "empty app state must not keep rename line-split target"
            );
            assert!(
                self.rename_tab_target.is_none(),
                "empty app state must not keep rename tab target"
            );
            assert!(
                self.selection.is_none(),
                "empty app state must not keep text selection"
            );
            assert!(
                self.selection_autoscroll.is_none(),
                "empty app state must not keep selection autoscroll"
            );
            if let Some(toast) = &self.toast {
                assert!(
                    toast.target.is_none(),
                    "empty app state must not keep pane-targeted toast"
                );
            }
            assert!(
                self.right_click_passthrough.is_none(),
                "empty app state must not keep right-click passthrough gesture"
            );
            assert!(
                self.drag.is_none(),
                "empty app state must not keep drag state"
            );
            assert!(
                self.workspace_press.is_none(),
                "empty app state must not keep workspace press state"
            );
            assert!(
                self.tab_press.is_none(),
                "empty app state must not keep tab press state"
            );
            assert!(
                self.agent_press.is_none(),
                "empty app state must not keep agent press state"
            );
            assert!(
                self.agent_manual_order.order.is_empty(),
                "empty app state must not keep manual agent order entries"
            );
            assert!(
                self.agent_manual_order.known.is_empty(),
                "empty app state must not keep known manual agent panes"
            );
            assert!(
                self.context_menu.is_none(),
                "empty app state must not keep context menu"
            );
            return;
        }

        assert!(
            self.selected < self.workspaces.len(),
            "selected workspace {} out of bounds for {} workspaces",
            self.selected,
            self.workspaces.len()
        );
        let active = self
            .active
            .expect("non-empty app state must have active workspace");
        assert!(
            active < self.workspaces.len(),
            "active workspace {} out of bounds for {} workspaces",
            active,
            self.workspaces.len()
        );

        let mut workspace_ids = std::collections::HashSet::new();
        let mut workspace_id_to_idx = std::collections::HashMap::new();
        let mut pane_ids = std::collections::HashSet::new();
        let mut attached_terminal_ids = std::collections::HashSet::new();
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            assert!(
                workspace_ids.insert(ws.id.clone()),
                "duplicate workspace id {} at workspace index {}",
                ws.id,
                ws_idx
            );
            workspace_id_to_idx.insert(ws.id.clone(), ws_idx);
            ws.assert_invariants_for_test();

            for tab in &ws.tabs {
                for (pane_id, pane) in &tab.panes {
                    assert!(
                        pane_ids.insert(*pane_id),
                        "pane {:?} appears in more than one workspace",
                        pane_id
                    );
                    assert!(
                        attached_terminal_ids.insert(pane.attached_terminal_id.clone()),
                        "terminal {} is attached to more than one app pane",
                        pane.attached_terminal_id
                    );
                    assert!(
                        self.terminals.contains_key(&pane.attached_terminal_id),
                        "pane {:?} is attached to missing terminal {}",
                        pane_id,
                        pane.attached_terminal_id
                    );
                }
            }
        }

        let assert_live_pane = |pane_id: PaneId, context: &str| {
            assert!(
                pane_ids.contains(&pane_id),
                "{context} references missing pane {:?}",
                pane_id
            );
        };
        let assert_workspace_pane = |workspace_id: &str, pane_id: PaneId, context: &str| {
            let ws_idx = workspace_id_to_idx
                .get(workspace_id)
                .copied()
                .unwrap_or_else(|| panic!("{context} references missing workspace {workspace_id}"));
            assert!(
                self.workspaces[ws_idx].pane_state(pane_id).is_some(),
                "{context} references pane {:?} outside workspace {}",
                pane_id,
                workspace_id
            );
        };
        let assert_workspace_index = |ws_idx: usize, context: &str| {
            assert!(
                ws_idx < self.workspaces.len(),
                "{context} references workspace index {} out of bounds for {} workspaces",
                ws_idx,
                self.workspaces.len()
            );
        };
        let assert_tab_index = |ws_idx: usize, tab_idx: usize, context: &str| {
            assert_workspace_index(ws_idx, context);
            assert!(
                tab_idx < self.workspaces[ws_idx].tabs.len(),
                "{context} references tab index {} out of bounds for workspace {} with {} tabs",
                tab_idx,
                ws_idx,
                self.workspaces[ws_idx].tabs.len()
            );
        };

        for (&raw, &pane_id) in &self.pane_id_aliases {
            assert_live_pane(pane_id, &format!("raw pane alias {raw}"));
        }
        for (public_id, &pane_id) in &self.public_pane_id_aliases {
            assert_live_pane(pane_id, &format!("public pane alias {public_id}"));
        }
        if let Some(focus) = &self.previous_pane_focus {
            assert_workspace_pane(&focus.workspace_id, focus.pane_id, "previous pane focus");
        }
        if let Some(toast) = &self.toast {
            if let Some(target) = &toast.target {
                assert_workspace_pane(&target.workspace_id, target.pane_id, "toast target");
            }
        }
        for (&pane_id, notification) in &self.pending_agent_notifications {
            assert_eq!(
                pane_id, notification.pane_id,
                "pending agent notification map key must match payload pane id"
            );
            assert_workspace_pane(
                &notification.workspace_id,
                notification.pane_id,
                "pending agent notification",
            );
        }
        for &pane_id in self.plugin_panes.keys() {
            assert_live_pane(pane_id, "plugin pane record");
        }
        if let Some(copy_mode) = &self.copy_mode {
            assert_live_pane(copy_mode.pane_id, "copy mode");
        }
        if let Some(pane_id) = self.rename_pane_target {
            assert_live_pane(pane_id, "rename pane target");
        }
        if let Some(selection) = &self.selection {
            assert_live_pane(selection.pane_id, "text selection");
        } else {
            assert!(
                self.selection_autoscroll.is_none(),
                "selection autoscroll must not remain without an active text selection"
            );
        }
        if let Some(gesture) = &self.right_click_passthrough {
            assert_live_pane(gesture.pane_info.id, "right-click passthrough gesture");
        }
        if let Some(drag) = &self.drag {
            match &drag.target {
                DragTarget::WorkspaceReorder {
                    source_ws_idx,
                    insert_idx,
                } => {
                    assert_workspace_index(*source_ws_idx, "workspace drag source");
                    if let Some(insert_idx) = insert_idx {
                        assert!(
                            *insert_idx <= self.workspaces.len(),
                            "workspace drag insert index {} out of bounds for {} workspaces",
                            insert_idx,
                            self.workspaces.len()
                        );
                    }
                }
                DragTarget::TabReorder {
                    ws_idx,
                    source_tab_idx,
                    insert_idx,
                } => {
                    assert_tab_index(*ws_idx, *source_tab_idx, "tab drag source");
                    if let Some(insert_idx) = insert_idx {
                        assert!(
                            *insert_idx <= self.workspaces[*ws_idx].tabs.len(),
                            "tab drag insert index {} out of bounds for workspace {} with {} tabs",
                            insert_idx,
                            ws_idx,
                            self.workspaces[*ws_idx].tabs.len()
                        );
                    }
                }
                DragTarget::PaneScrollbar { pane_id, .. } => {
                    assert_live_pane(*pane_id, "pane scrollbar drag")
                }
                DragTarget::AgentReorder { source, insert_idx } => {
                    if let ManualEntryRef::Pane(source_pane_id) = source {
                        assert_live_pane(*source_pane_id, "agent reorder drag source");
                    }
                    if let Some(insert_idx) = insert_idx {
                        assert!(
                            *insert_idx <= self.agent_manual_order.order.len(),
                            "agent reorder insert index {} out of bounds for {} manual entries",
                            insert_idx,
                            self.agent_manual_order.order.len()
                        );
                    }
                }
                _ => {}
            }
        }
        if let Some(press) = &self.workspace_press {
            assert_workspace_index(press.ws_idx, "workspace press");
        }
        if let Some(press) = &self.tab_press {
            assert_tab_index(press.ws_idx, press.tab_idx, "tab press");
        }
        if let Some(press) = &self.agent_press {
            if let ManualEntryRef::Pane(pane_id) = press.entry {
                assert_live_pane(pane_id, "agent press");
            }
        }
        let mut seen_line_split_ids = std::collections::HashSet::new();
        for entry in &self.agent_manual_order.order {
            match entry {
                ManualEntry::Pane(pane_id) => {
                    assert_live_pane(*pane_id, "manual agent order");
                    assert!(
                        self.agent_manual_order.known.contains(pane_id),
                        "manual agent order entry {:?} missing from known set",
                        pane_id
                    );
                }
                ManualEntry::LineSplit { id, .. } => {
                    assert!(
                        id.0 < self.agent_manual_order.next_line_split_id,
                        "line-split id {} not below next_line_split_id {}",
                        id.0,
                        self.agent_manual_order.next_line_split_id
                    );
                    assert!(
                        seen_line_split_ids.insert(id.0),
                        "duplicate line-split id {} in manual agent order",
                        id.0
                    );
                }
            }
        }
        for pane_id in &self.agent_manual_order.known {
            assert_live_pane(*pane_id, "known manual agent pane");
        }
        let mut seen_pane_split_ids = std::collections::HashSet::new();
        for entry in &self.pane_section_order.order {
            if let PaneManualEntry::LineSplit { id, .. } = entry {
                assert!(
                    id.0 < self.pane_section_order.next_line_split_id,
                    "pane-section line-split id {} not below next_line_split_id {}",
                    id.0,
                    self.pane_section_order.next_line_split_id
                );
                assert!(
                    seen_pane_split_ids.insert(id.0),
                    "duplicate line-split id {} in pane-section order",
                    id.0
                );
            }
        }
        if self.mode != Mode::RenameLineSplit {
            assert!(
                self.rename_line_split_target.is_none(),
                "rename line-split target must be cleared outside RenameLineSplit mode"
            );
        }
        if self.mode != Mode::RenameTab {
            assert!(
                self.rename_tab_target.is_none(),
                "rename tab target must be cleared outside RenameTab mode"
            );
        } else if let Some((ws_idx, tab_idx)) = self.rename_tab_target {
            assert!(
                self.workspaces
                    .get(ws_idx)
                    .is_some_and(|ws| tab_idx < ws.tabs.len()),
                "rename tab target must reference a live tab"
            );
        }
        if let Some(menu) = &self.context_menu {
            match menu.kind {
                ContextMenuKind::Workspace { ws_idx }
                | ContextMenuKind::GitWorkspace { ws_idx, .. } => {
                    assert_workspace_index(ws_idx, "context menu workspace")
                }
                ContextMenuKind::Tab { ws_idx, tab_idx } => {
                    assert_tab_index(ws_idx, tab_idx, "context menu tab")
                }
                ContextMenuKind::Pane {
                    ws_idx,
                    tab_idx,
                    pane_id,
                    source_pane_id,
                    ..
                } => {
                    assert_tab_index(ws_idx, tab_idx, "context menu pane tab");
                    assert!(
                        self.workspaces[ws_idx].tabs[tab_idx]
                            .panes
                            .contains_key(&pane_id),
                        "context menu pane references pane {:?} outside workspace {} tab {}",
                        pane_id,
                        ws_idx,
                        tab_idx
                    );
                    if let Some(source_pane_id) = source_pane_id {
                        assert_live_pane(source_pane_id, "context menu source pane");
                    }
                }
                ContextMenuKind::LineSplit { section, id } => match section {
                    LineSplitSection::Agents => assert!(
                        self.agent_manual_order
                            .order
                            .iter()
                            .any(|entry| matches!(entry, ManualEntry::LineSplit { id: entry_id, .. } if *entry_id == id)),
                        "context menu references agents line-split {:?} not present in manual order",
                        id
                    ),
                    LineSplitSection::Panes => assert!(
                        self.pane_section_order
                            .order
                            .iter()
                            .any(|entry| matches!(entry, PaneManualEntry::LineSplit { id: entry_id, .. } if *entry_id == id)),
                        "context menu references panes line-split {:?} not present in pane-section order",
                        id
                    ),
                },
            }
        }
    }

    pub fn insert_test_runtime(
        &mut self,
        pane_id: crate::layout::PaneId,
        runtime: crate::terminal::TerminalRuntime,
    ) {
        if let Some(ws) = self
            .workspaces
            .iter_mut()
            .find(|ws| ws.terminal_id(pane_id).is_some())
        {
            ws.insert_test_runtime(pane_id, runtime);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn agent_terminal_keeps_final_child_cursor_exposed() {
        let mut state = AppState::test_new();
        let ws = crate::workspace::Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        state.terminals.insert(
            ws.tabs[0].panes[&pane_id].attached_terminal_id.clone(),
            crate::terminal::TerminalState::new(
                ws.tabs[0].panes[&pane_id].attached_terminal_id.clone(),
                std::path::PathBuf::from("/tmp"),
            ),
        );
        state
            .terminals
            .get_mut(&ws.tabs[0].panes[&pane_id].attached_terminal_id)
            .expect("terminal state")
            .launch_argv = Some(vec!["codex".to_string()]);
        state.workspaces = vec![ws];

        assert!(state.pane_exposes_host_cursor(0, pane_id));
    }

    #[test]
    fn adversarial_identity_state_satisfies_app_invariants_after_mutation() {
        let mut state = AppState::test_with_adversarial_identity_state();
        state.assert_invariants_for_test();

        let ws = &mut state.workspaces[0];
        let active_public = ws.tabs[ws.active_tab].number;
        assert_ne!(ws.active_tab + 1, active_public);
        let new_pane = ws.test_split(ratatui::layout::Direction::Horizontal);
        assert!(ws.public_pane_number(new_pane).is_some());
        state.ensure_test_terminals();

        state.assert_invariants_for_test();
    }

    #[test]
    fn parent_link_on_adversarial_state_preserves_identity_invariants() {
        let mut state = AppState::test_with_adversarial_identity_state();
        // Link the newly split pane to the tab's root by stable public number,
        // as `agent start --parent` does, against identity-scrambled state.
        let ws = &mut state.workspaces[0];
        let root = ws.tabs[ws.active_tab].root_pane;
        let child = ws.test_split(ratatui::layout::Direction::Horizontal);
        let parent_number = ws.public_pane_number(root).expect("root has number");
        let workspace_id = ws.id.clone();
        ws.pane_state_mut(child).expect("child pane").parent = Some(crate::pane::PaneParentRef {
            workspace_id: workspace_id.clone(),
            pane_number: parent_number,
        });
        state.ensure_test_terminals();

        state.assert_invariants_for_test();

        // The stable parent link resolves back to the root pane.
        let parent_ref = crate::pane::PaneParentRef {
            workspace_id,
            pane_number: parent_number,
        };
        assert_eq!(state.resolve_pane_parent(&parent_ref), Some((0, root)));
    }

    #[test]
    fn built_in_theme_names_resolve() {
        for name in THEME_NAMES {
            assert!(
                Palette::from_name(name).is_some(),
                "theme should resolve: {name}"
            );
        }
    }

    #[test]
    fn light_theme_aliases_resolve() {
        for name in ["light", "latte", "tokyo-day", "onelight", "lotus", "dawn"] {
            assert!(
                Palette::from_name(name).is_some(),
                "theme should resolve: {name}"
            );
        }
    }

    #[test]
    fn key_matches_requires_exact_modifiers() {
        assert!(key_matches(
            &KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));

        assert!(!key_matches(
            &KeyEvent::new(
                KeyCode::Char('b'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));
    }

    #[test]
    fn key_matches_letters_case_insensitively() {
        assert!(key_matches(
            &KeyEvent::new(KeyCode::Char('B'), KeyModifiers::SHIFT),
            KeyCode::Char('b'),
            KeyModifiers::SHIFT,
        ));
    }

    #[test]
    fn linked_worktree_context_menu_keeps_safe_close_and_explicit_remove() {
        let menu = ContextMenuState {
            kind: ContextMenuKind::GitWorkspace {
                ws_idx: 0,
                is_linked_worktree: true,
                has_worktree_children: false,
                collapsed: false,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };

        assert_eq!(
            menu.items(),
            &["Rename", "Close", "Delete worktree checkout..."]
        );
    }

    #[test]
    fn git_workspace_context_menu_keeps_remove_for_managed_worktrees_only() {
        let menu = ContextMenuState {
            kind: ContextMenuKind::GitWorkspace {
                ws_idx: 0,
                is_linked_worktree: false,
                has_worktree_children: false,
                collapsed: false,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };

        assert_eq!(
            menu.items(),
            &["Rename", "Close", "New worktree", "Open worktree..."]
        );
    }

    #[test]
    fn parent_worktree_context_menu_uses_repo_actions() {
        let menu = ContextMenuState {
            kind: ContextMenuKind::GitWorkspace {
                ws_idx: 0,
                is_linked_worktree: false,
                has_worktree_children: true,
                collapsed: false,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };

        assert_eq!(
            menu.items(),
            &[
                "Rename",
                "Close group",
                "New worktree",
                "Open worktree...",
                "Collapse"
            ]
        );
    }
}
