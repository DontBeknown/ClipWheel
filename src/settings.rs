use std::path::PathBuf;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub key: u32,
}
impl Default for Hotkey {
    fn default() -> Self {
        Self {
            modifiers: 3,
            key: 0x56,
        }
    }
}
impl Hotkey {
    pub fn parse(text: &str) -> Option<Self> {
        let mut modifiers = 0;
        let mut key = None;
        for token in text
            .trim()
            .split('+')
            .map(|s| s.trim().to_ascii_uppercase())
        {
            match token.as_str() {
                "CTRL" => modifiers |= 2,
                "ALT" => modifiers |= 1,
                "SHIFT" => modifiers |= 4,
                s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(s.as_bytes()[0] as u32);
                }
                _ => return None,
            }
        }
        if modifiers == 0 {
            return None;
        }
        Some(Self {
            modifiers,
            key: key?,
        })
    }
    pub fn label(self) -> String {
        let mut s = String::new();
        for (bit, label) in [(2, "Ctrl+"), (1, "Alt+"), (4, "Shift+")] {
            if self.modifiers & bit != 0 {
                s.push_str(label);
            }
        }
        s.push(char::from_u32(self.key).unwrap_or('V'));
        s
    }
}
pub fn path() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()))
        .join("ClipWheel/settings.txt")
}
pub fn load() -> Hotkey {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| Hotkey::parse(&s))
        .unwrap_or_default()
}
pub fn save(key: Hotkey) -> std::io::Result<()> {
    let p = path();
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(p, key.label())
}
