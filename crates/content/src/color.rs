use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour, written in content files as `"#rrggbb"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// Multiplies each channel by `factor`, clamping to the valid range.
    pub fn scale(self, factor: f32) -> Self {
        let f = |c: u8| (c as f32 * factor).round().clamp(0.0, 255.0) as u8;
        Rgb(f(self.0), f(self.1), f(self.2))
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2))
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid colour {s:?}, expected \"#rrggbb\"")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colours() {
        assert_eq!(Rgb::parse("#ff8000"), Some(Rgb(255, 128, 0)));
        assert_eq!(Rgb::parse("ff8000"), None);
        assert_eq!(Rgb::parse("#ff80"), None);
    }
}
