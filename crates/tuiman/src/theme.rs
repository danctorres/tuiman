//! Colour themes. `default` uses the terminal's own palette; the others paint
//! their own background in 24-bit colour.

use ratatui::style::{Color, Style};

pub struct Theme {
    pub name: &'static str,
    /// Base foreground and background, painted under everything.
    pub fg: Color,
    pub bg: Color,
    pub dim: Color,
    pub accent: Color,
    pub stars: Color,
    pub installed: Color,
    pub archived: Color,
    pub library: Color,
    pub link: Color,
}

impl Theme {
    pub fn base(&self) -> Style {
        Style::new().fg(self.fg).bg(self.bg)
    }

    pub fn dim(&self) -> Style {
        Style::new().fg(self.dim)
    }

    pub fn accent(&self) -> Style {
        Style::new().fg(self.accent)
    }

    pub fn selected(&self) -> Style {
        let fg = if self.bg == Color::Reset { Color::Black } else { self.bg };
        Style::new().fg(fg).bg(self.accent)
    }
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const THEMES: &[Theme] = &[
    Theme {
        name: "default",
        fg: Color::Reset,
        bg: Color::Reset,
        dim: Color::DarkGray,
        accent: Color::Cyan,
        stars: Color::Yellow,
        installed: Color::Green,
        archived: Color::Red,
        library: Color::Magenta,
        link: Color::Blue,
    },
    Theme {
        name: "gruvbox",
        fg: rgb(0xebdbb2),
        bg: rgb(0x282828),
        dim: rgb(0x928374),
        accent: rgb(0x8ec07c),
        stars: rgb(0xfabd2f),
        installed: rgb(0xb8bb26),
        archived: rgb(0xfb4934),
        library: rgb(0xd3869b),
        link: rgb(0x83a598),
    },
    Theme {
        name: "nord",
        fg: rgb(0xd8dee9),
        bg: rgb(0x2e3440),
        dim: rgb(0x616e88),
        accent: rgb(0x88c0d0),
        stars: rgb(0xebcb8b),
        installed: rgb(0xa3be8c),
        archived: rgb(0xbf616a),
        library: rgb(0xb48ead),
        link: rgb(0x81a1c1),
    },
    Theme {
        name: "catppuccin",
        fg: rgb(0xcdd6f4),
        bg: rgb(0x1e1e2e),
        dim: rgb(0x6c7086),
        accent: rgb(0xcba6f7),
        stars: rgb(0xf9e2af),
        installed: rgb(0xa6e3a1),
        archived: rgb(0xf38ba8),
        library: rgb(0xf5c2e7),
        link: rgb(0x89b4fa),
    },
    Theme {
        name: "tokyonight",
        fg: rgb(0xc0caf5),
        bg: rgb(0x1a1b26),
        dim: rgb(0x565f89),
        accent: rgb(0x7dcfff),
        stars: rgb(0xe0af68),
        installed: rgb(0x9ece6a),
        archived: rgb(0xf7768e),
        library: rgb(0xbb9af7),
        link: rgb(0x7aa2f7),
    },
    Theme {
        name: "dracula",
        fg: rgb(0xf8f8f2),
        bg: rgb(0x282a36),
        dim: rgb(0x6272a4),
        accent: rgb(0x8be9fd),
        stars: rgb(0xf1fa8c),
        installed: rgb(0x50fa7b),
        archived: rgb(0xff5555),
        library: rgb(0xff79c6),
        link: rgb(0xbd93f9),
    },
    Theme {
        name: "solarized-light",
        fg: rgb(0x586e75),
        bg: rgb(0xfdf6e3),
        dim: rgb(0x93a1a1),
        accent: rgb(0x2aa198),
        stars: rgb(0xb58900),
        installed: rgb(0x859900),
        archived: rgb(0xdc322f),
        library: rgb(0xd33682),
        link: rgb(0x268bd2),
    },
    Theme {
        name: "rose-pine",
        fg: rgb(0xe0def4),
        bg: rgb(0x191724),
        dim: rgb(0x6e6a86),
        accent: rgb(0xebbcba),
        stars: rgb(0xf6c177),
        installed: rgb(0x9ccfd8),
        archived: rgb(0xeb6f92),
        library: rgb(0xc4a7e7),
        link: rgb(0x31748f),
    },
    Theme {
        name: "kanagawa",
        fg: rgb(0xdcd7ba),
        bg: rgb(0x1f1f28),
        dim: rgb(0x727169),
        accent: rgb(0x7aa89f),
        stars: rgb(0xe6c384),
        installed: rgb(0x98bb6c),
        archived: rgb(0xff5d62),
        library: rgb(0x957fb8),
        link: rgb(0x7e9cd8),
    },
    Theme {
        name: "everforest",
        fg: rgb(0xd3c6aa),
        bg: rgb(0x2d353b),
        dim: rgb(0x859289),
        accent: rgb(0x83c092),
        stars: rgb(0xdbbc7f),
        installed: rgb(0xa7c080),
        archived: rgb(0xe67e80),
        library: rgb(0xd699b6),
        link: rgb(0x7fbbb3),
    },
    Theme {
        name: "monokai",
        fg: rgb(0xf8f8f2),
        bg: rgb(0x272822),
        dim: rgb(0x75715e),
        accent: rgb(0xfd971f),
        stars: rgb(0xe6db74),
        installed: rgb(0xa6e22e),
        archived: rgb(0xf92672),
        library: rgb(0xae81ff),
        link: rgb(0x66d9ef),
    },
    Theme {
        name: "synthwave",
        fg: rgb(0xffffff),
        bg: rgb(0x262335),
        dim: rgb(0x848bbd),
        accent: rgb(0xff7edb),
        stars: rgb(0xfede5d),
        installed: rgb(0x72f1b8),
        archived: rgb(0xfe4450),
        library: rgb(0xf97e72),
        link: rgb(0x36f9f6),
    },
    Theme {
        name: "paper",
        fg: rgb(0x1a1a1a),
        bg: rgb(0xffffff),
        dim: rgb(0x8a8a8a),
        accent: rgb(0x000000),
        stars: rgb(0xb8860b),
        installed: rgb(0x2e7d32),
        archived: rgb(0xc62828),
        library: rgb(0x6a1b9a),
        link: rgb(0x1565c0),
    },
    Theme {
        name: "bubblegum",
        fg: rgb(0x5a2a4a),
        bg: rgb(0xffe4f1),
        dim: rgb(0xb48aa6),
        accent: rgb(0xd6336c),
        stars: rgb(0xc08400),
        installed: rgb(0x2a9d8f),
        archived: rgb(0xd62828),
        library: rgb(0x9b5de5),
        link: rgb(0x3a86ff),
    },
    Theme {
        name: "gameboy",
        fg: rgb(0x0f380f),
        bg: rgb(0x9bbc0f),
        dim: rgb(0x306230),
        accent: rgb(0x306230),
        stars: rgb(0x0f380f),
        installed: rgb(0x0f380f),
        archived: rgb(0x0f380f),
        library: rgb(0x306230),
        link: rgb(0x306230),
    },
    Theme {
        name: "phosphor",
        fg: rgb(0x33ff66),
        bg: rgb(0x0a0f0a),
        dim: rgb(0x1a7f33),
        accent: rgb(0x33ff66),
        stars: rgb(0xccffcc),
        installed: rgb(0x99ffaa),
        archived: rgb(0x0f9f3f),
        library: rgb(0x66ff99),
        link: rgb(0x66ff99),
    },
    Theme {
        name: "amber",
        fg: rgb(0xffb000),
        bg: rgb(0x1a1000),
        dim: rgb(0x7f5800),
        accent: rgb(0xffcc00),
        stars: rgb(0xffe8a3),
        installed: rgb(0xffd966),
        archived: rgb(0xcc5500),
        library: rgb(0xff9900),
        link: rgb(0xffc34d),
    },
    Theme {
        name: "high-contrast",
        fg: rgb(0xffffff),
        bg: rgb(0x000000),
        dim: rgb(0xaaaaaa),
        accent: rgb(0xffff00),
        stars: rgb(0xffff00),
        installed: rgb(0x00ff00),
        archived: rgb(0xff0000),
        library: rgb(0xff00ff),
        link: rgb(0x00ffff),
    },
];

/// Index of the theme called `name`, falling back to `default`.
pub fn by_name(name: &str) -> usize {
    THEMES.iter().position(|t| t.name == name.trim()).unwrap_or(0)
}
