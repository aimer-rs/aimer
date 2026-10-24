#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub enum Colors {
    Red,
    Green,
    Blue,
    White,
    #[default]
    Black,
    Yellow,
    Cyan,
    Magenta,
    Gray,
    Orange,
    Purple,
    Brown,
    Transparent,
    Rgba(u8, u8, u8, u8),
    Rgb(u8, u8, u8),
    Custom(u32),
}

impl Colors {
    pub const fn alpha(&self, index: u8) -> Self {
        let base_u32 = self.as_u32();
        let r = (base_u32 >> 16) & 0xFF;
        let g = (base_u32 >> 8) & 0xFF;
        let b = base_u32 & 0xFF;
        let alpha = index;

        let argb = ((alpha as u32) << 24) | (r << 16) | (g << 8) | b;

        match argb {
            0xFFFF0000 => Colors::Red,
            0xFF00FF00 => Colors::Green,
            0xFF0000FF => Colors::Blue,
            0xFFFFFFFF => Colors::White,
            0xFF000000 => Colors::Black,
            0xFFFFFF00 => Colors::Yellow,
            0xFF00FFFF => Colors::Cyan,
            0xFFFF00FF => Colors::Magenta,
            0xFF808080 => Colors::Gray,
            0xFFFFA500 => Colors::Orange,
            0xFF800080 => Colors::Purple,
            0xFFA52A2A => Colors::Brown,
            0x00000000 => Colors::Transparent,
            _ => Colors::Custom(argb),
        }
    }

    pub const fn as_u32(&self) -> u32 {
        match self {
            Colors::Red => 0xFFFF0000,
            Colors::Green => 0xFF00FF00,
            Colors::Blue => 0xFF0000FF,
            Colors::White => 0xFFFFFFFF,
            Colors::Black => 0xFF000000,
            Colors::Yellow => 0xFFFFFF00,
            Colors::Cyan => 0xFF00FFFF,
            Colors::Magenta => 0xFFFF00FF,
            Colors::Gray => 0xFF808080,
            Colors::Orange => 0xFFFFA500,
            Colors::Purple => 0xFF800080,
            Colors::Brown => 0xFFA52A2A,
            Colors::Transparent => 0x00000000,
            Colors::Custom(c) => *c,
            Colors::Rgba(r, g, b, a) => {
                ((*a as u32) << 24) | ((*r as u32) << 16) | ((*g as u32) << 8) | (*b as u32)
            }
            Colors::Rgb(r, g, b) => {
                (0xFF << 24) | ((*r as u32) << 16) | ((*g as u32) << 8) | (*b as u32)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_more_colors() {
        assert_eq!(Colors::Orange.as_u32(), 0xFFFFA500);
        assert_eq!(Colors::Purple.as_u32(), 0xFF800080);
        assert_eq!(Colors::Brown.as_u32(), 0xFFA52A2A);
        assert_eq!(Colors::Transparent.as_u32(), 0x00000000);
    }
}
