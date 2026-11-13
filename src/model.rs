use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Payload {
    Text(String),
    Image {
        width: u32,
        height: u32,
        bgra: Vec<u8>,
    },
    Files(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u64,
    pub payload: Arc<Payload>,
    pub preview: String,
    pub bytes: usize,
    pub thumbnail: Option<Thumbnail>,
    pub details: Details,
    fingerprint: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Details {
    file_text: String,
    starts: Vec<usize>,
}
impl Entry {
    pub fn section_count(&self) -> usize {
        self.details.starts.len().max(1)
    }
    pub fn section(&self, index: usize) -> &str {
        let text = match self.payload.as_ref() {
            Payload::Text(s) => s.as_str(),
            Payload::Files(_) => &self.details.file_text,
            _ => "",
        };
        let start = self.details.starts.get(index).copied().unwrap_or(0);
        let end = self
            .details
            .starts
            .get(index + 1)
            .copied()
            .unwrap_or(text.len());
        &text[start..end]
    }
}

#[derive(Clone, Debug)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub struct Prepared {
    pub payload: Payload,
    pub thumbnail: Option<Thumbnail>,
    pub details: Details,
    fingerprint: u64,
}
impl Prepared {
    pub fn new(payload: Payload) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        payload.hash(&mut hasher);
        let fingerprint = hasher.finish();
        let thumbnail = if let Payload::Image {
            width,
            height,
            bgra,
        } = &payload
        {
            if *width == 0
                || *height == 0
                || (*width as usize)
                    .checked_mul(*height as usize)
                    .and_then(|n| n.checked_mul(4))
                    != Some(bgra.len())
            {
                None
            } else {
                let ratio = (512. / (*width).max(*height) as f32).min(1.);
                let w = (*width as f32 * ratio).max(1.) as u32;
                let h = (*height as f32 * ratio).max(1.) as u32;
                let mut pixels = vec![0; (w * h * 4) as usize];
                for y in 0..h {
                    for x in 0..w {
                        let sx = x as u64 * *width as u64 / w as u64;
                        let sy = y as u64 * *height as u64 / h as u64;
                        let from = ((sy * *width as u64 + sx) * 4) as usize;
                        let to = ((y * w + x) * 4) as usize;
                        pixels[to..to + 4].copy_from_slice(&bgra[from..from + 4]);
                        for c in 0..3 {
                            pixels[to + c] =
                                (pixels[to + c] as u16 * pixels[to + 3] as u16 / 255) as u8;
                        }
                    }
                }
                Some(Thumbnail {
                    width: w,
                    height: h,
                    bgra: pixels,
                })
            }
        } else {
            None
        };
        let file_text = if let Payload::Files(paths) = &payload {
            paths.join("\n\n")
        } else {
            String::new()
        };
        let text = if let Payload::Text(s) = &payload {
            s.as_str()
        } else {
            file_text.as_str()
        };
        let mut starts = vec![0];
        let mut start = 0;
        while text.len() - start > 4096 {
            let mut end = start + 4096;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            starts.push(end);
            start = end;
        }
        Self {
            payload,
            thumbnail,
            fingerprint,
            details: Details { file_text, starts },
        }
    }
}

pub struct History {
    pub entries: Vec<Arc<Entry>>,
    pub bytes: usize,
    count_limit: usize,
    byte_limit: usize,
    item_limit: usize,
    next_id: u64,
}

impl History {
    pub fn new(count_limit: usize, byte_limit: usize, item_limit: usize) -> Self {
        Self {
            entries: vec![],
            bytes: 0,
            count_limit,
            byte_limit,
            item_limit,
            next_id: 1,
        }
    }
    pub fn insert(&mut self, payload: Payload) -> Result<(), &'static str> {
        self.insert_prepared(Prepared::new(payload))
    }
    pub fn insert_prepared(&mut self, prepared: Prepared) -> Result<(), &'static str> {
        self.insert_pinned(prepared, &[])
    }
    pub fn insert_pinned(
        &mut self,
        prepared: Prepared,
        pinned: &[Arc<Entry>],
    ) -> Result<(), &'static str> {
        let Prepared {
            payload,
            thumbnail,
            details,
            fingerprint,
        } = prepared;
        let payload_bytes = match &payload {
            Payload::Text(s) => s.len(),
            Payload::Image { bgra, .. } => bgra.len(),
            Payload::Files(paths) => paths.iter().map(|s| s.len()).sum(),
        };
        let preview: String = match &payload {
            Payload::Text(s) => s
                .chars()
                .take(80)
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect(),
            Payload::Files(paths) => paths
                .first()
                .map(|s| {
                    s.rsplit(['\\', '/'])
                        .next()
                        .unwrap_or(s)
                        .chars()
                        .take(80)
                        .collect()
                })
                .unwrap_or_default(),
            Payload::Image { width, height, .. } => format!("{width} × {height}"),
        };
        let bytes = payload_bytes
            .saturating_add(preview.len())
            .saturating_add(thumbnail.as_ref().map_or(0, |t| t.bgra.len()))
            .saturating_add(details.file_text.len())
            .saturating_add(details.starts.len() * std::mem::size_of::<usize>());
        if bytes > self.item_limit || bytes > self.byte_limit {
            return Err("Item exceeds the clipboard memory limit");
        }
        if self.count_limit == 0 {
            return Err("History is disabled");
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.fingerprint == fingerprint && *e.payload == payload)
        {
            let entry = self.entries.remove(index);
            self.entries.insert(0, entry);
            return Ok(());
        }
        let entry = if let Some(existing) = pinned
            .iter()
            .find(|e| e.fingerprint == fingerprint && *e.payload == payload)
        {
            existing.clone()
        } else {
            let entry = Arc::new(Entry {
                id: self.next_id,
                payload: Arc::new(payload),
                preview,
                bytes,
                thumbnail,
                details,
                fingerprint,
            });
            self.next_id += 1;
            entry
        };
        let mut candidate = self.entries.clone();
        candidate.insert(0, entry);
        let union_bytes = |entries: &[Arc<Entry>]| -> usize {
            entries.iter().map(|e| e.bytes).sum::<usize>()
                + pinned
                    .iter()
                    .filter(|p| !entries.iter().any(|e| e.id == p.id))
                    .map(|e| e.bytes)
                    .sum::<usize>()
        };
        while candidate.len() > 1
            && (candidate.len() > self.count_limit || union_bytes(&candidate) > self.byte_limit)
        {
            candidate.pop();
        }
        if union_bytes(&candidate) > self.byte_limit {
            return Err(
                "History memory is full while the wheel is open. Close it to allow eviction.",
            );
        }
        self.bytes = candidate.iter().map(|e| e.bytes).sum();
        self.entries = candidate;
        Ok(())
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}
