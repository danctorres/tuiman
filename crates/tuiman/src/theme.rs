//! Colour themes. `default` uses the terminal's own palette; the others paint
//! their own background in 24-bit colour.

use ratatui::style::{Color, Modifier, Style};

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

    /// `dim` text lit up towards bold `accent` by `level`, 0 to 1.
    pub fn flash(&self, level: f32) -> Style {
        let style = Style::new().fg(mix(self.dim, self.accent, level));
        if level >= 0.5 {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }

    /// The faint bar behind a category's count. `None` where a theme on the
    /// terminal palette cannot blend, and so goes without.
    pub fn meter(&self) -> Option<Style> {
        match mix(self.bg, self.accent, 0.18) {
            Color::Rgb(r, g, b) => Some(Style::new().bg(Color::Rgb(r, g, b))),
            _ => None,
        }
    }

    /// `colour` sunk most of the way towards the background, for the backdrop
    /// behind a modal. `None` when either end is not 24-bit and cannot blend.
    pub fn sink(&self, colour: Color) -> Option<Color> {
        match (colour, self.bg) {
            (Color::Rgb(..), Color::Rgb(..)) => Some(mix(colour, self.bg, 0.6)),
            _ => None,
        }
    }

    /// A point on the accent-to-link sweep that borders and the selection bar
    /// ride, or `None` for a theme on the terminal palette, which cannot blend
    /// and would band into two flat halves instead.
    pub fn sweep(&self, level: f32) -> Option<Color> {
        match (self.accent, self.link) {
            (Color::Rgb(..), Color::Rgb(..)) => Some(mix(self.accent, self.link, level)),
            _ => None,
        }
    }

    /// The star count's colour, warming from `dim` to `stars` on a log scale
    /// so popularity reads at a glance: ~100 stars is cold, 30k is full heat.
    /// A palette that cannot blend keeps every count at full heat, rather
    /// than snapping half the table to the dim colour at some arbitrary count.
    pub fn heat(&self, stars: Option<u32>) -> Style {
        let level = match stars {
            Some(n) if n > 0 => (((n as f32).log10() - 2.0) / 2.5).clamp(0.0, 1.0),
            _ => 0.0,
        };
        match (self.dim, self.stars) {
            (Color::Rgb(..), Color::Rgb(..)) => Style::new().fg(mix(self.dim, self.stars, level)),
            _ => Style::new().fg(self.stars),
        }
    }

    /// The background of every other table row: the base background nudged
    /// towards the foreground. Named palettes cannot blend, so they get none.
    pub fn stripe(&self) -> Style {
        match mix(self.bg, self.fg, 0.06) {
            Color::Rgb(r, g, b) => Style::new().bg(Color::Rgb(r, g, b)),
            _ => Style::new(),
        }
    }

    pub fn selected(&self) -> Style {
        let fg = if self.bg == Color::Reset { Color::Black } else { self.bg };
        Style::new().fg(fg).bg(self.accent)
    }

    /// The selection bar while a search is typed, where enter only closes the
    /// search: a dead grey, so it does not read as ready to act on.
    pub fn searching(&self) -> Style {
        self.selected().bg(self.dim)
    }
}

/// Blends 24-bit colours; named ones cannot blend, so they switch halfway.
fn mix(from: Color, to: Color, level: f32) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let at = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * level).round() as u8;
            Color::Rgb(at(r1, r2), at(g1, g2), at(b1, b2))
        }
        _ if level < 0.5 => from,
        _ => to,
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
    // Dark.
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
        name: "github",
        fg: rgb(0xe6edf3),
        bg: rgb(0x0d1117),
        dim: rgb(0x7d8590),
        accent: rgb(0xa371f7),
        stars: rgb(0xd29922),
        installed: rgb(0x3fb950),
        archived: rgb(0xf85149),
        library: rgb(0xdb61a2),
        link: rgb(0x58a6ff),
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
        name: "solarized-dark",
        fg: rgb(0x839496),
        bg: rgb(0x002b36),
        dim: rgb(0x586e75),
        accent: rgb(0x2aa198),
        stars: rgb(0xb58900),
        installed: rgb(0x859900),
        archived: rgb(0xdc322f),
        library: rgb(0xd33682),
        link: rgb(0x268bd2),
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
    // Light.
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
        name: "catppuccin-latte",
        fg: rgb(0x4c4f69),
        bg: rgb(0xeff1f5),
        dim: rgb(0x9ca0b0),
        accent: rgb(0x8839ef),
        stars: rgb(0xdf8e1d),
        installed: rgb(0x40a02b),
        archived: rgb(0xd20f39),
        library: rgb(0xea76cb),
        link: rgb(0x1e66f5),
    },
    Theme {
        name: "github-light",
        fg: rgb(0x1f2328),
        bg: rgb(0xffffff),
        dim: rgb(0x6e7781),
        accent: rgb(0x8250df),
        stars: rgb(0x9a6700),
        installed: rgb(0x1a7f37),
        archived: rgb(0xcf222e),
        library: rgb(0xbf3989),
        link: rgb(0x0969da),
    },
    Theme {
        name: "gruvbox-light",
        fg: rgb(0x3c3836),
        bg: rgb(0xfbf1c7),
        dim: rgb(0x928374),
        accent: rgb(0x427b58),
        stars: rgb(0xb57614),
        installed: rgb(0x79740e),
        archived: rgb(0x9d0006),
        library: rgb(0x8f3f71),
        link: rgb(0x076678),
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
        name: "sepia",
        fg: rgb(0x5b4636),
        bg: rgb(0xf4ecd8),
        dim: rgb(0xa08c74),
        accent: rgb(0x8b4513),
        stars: rgb(0x9a6a00),
        installed: rgb(0x5f7a2e),
        archived: rgb(0xa33a2a),
        library: rgb(0x7b4a7a),
        link: rgb(0x2f5f8a),
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
    // Retro terminals and machines.
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
        name: "c64",
        fg: rgb(0xb8b0ff),
        bg: rgb(0x40318d),
        dim: rgb(0x7869c4),
        accent: rgb(0xffffff),
        stars: rgb(0xb8c76f),
        installed: rgb(0x9ae29b),
        archived: rgb(0xe8958c),
        library: rgb(0xd08ad3),
        link: rgb(0x6abfc6),
    },
    Theme {
        name: "cga",
        fg: rgb(0xffffff),
        bg: rgb(0x000000),
        dim: rgb(0x00aaaa),
        accent: rgb(0xff55ff),
        stars: rgb(0x55ffff),
        installed: rgb(0x55ffff),
        archived: rgb(0xff55ff),
        library: rgb(0xff55ff),
        link: rgb(0x55ffff),
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
    // Accessibility.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_warms_with_stars_and_stripes_need_rgb() {
        let t = &THEMES[by_name("nord")];
        let cold = t.heat(None).fg.unwrap();
        assert_eq!(cold, t.dim, "unknown and tiny counts stay dim");
        assert_eq!(t.heat(Some(100)).fg.unwrap(), t.dim);
        assert_eq!(t.heat(Some(50_000)).fg.unwrap(), t.stars, "a popular project is full heat");
        let mid = t.heat(Some(2_000)).fg.unwrap();
        assert!(mid != t.dim && mid != t.stars, "in between it blends: {mid:?}");

        assert!(t.stripe().bg.is_some(), "a 24-bit theme stripes its rows");
        let terminal = &THEMES[0];
        assert_eq!(terminal.heat(Some(50)).fg, Some(terminal.stars), "no blend, no fade");
        assert_eq!(terminal.stripe().bg, None, "the terminal palette cannot blend, so no stripe");
    }

    #[test]
    fn mix_blends_rgb_and_switches_named_colours() {
        let (black, white) = (Color::Rgb(0, 0, 0), Color::Rgb(200, 100, 50));
        assert_eq!(mix(black, white, 0.0), black);
        assert_eq!(mix(black, white, 0.5), Color::Rgb(100, 50, 25));
        assert_eq!(mix(black, white, 1.0), white);
        assert_eq!(mix(Color::Reset, Color::Cyan, 0.4), Color::Reset);
        assert_eq!(mix(Color::Reset, Color::Cyan, 0.6), Color::Cyan);
    }
}
