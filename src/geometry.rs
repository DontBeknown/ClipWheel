pub const PAGE_SIZE: usize = 8;
pub const INNER_RADIUS: f32 = 28.;
pub const OUTER_RADIUS: f32 = 140.;
pub fn pages(count: usize) -> usize {
    count.div_ceil(PAGE_SIZE).max(1)
}
pub fn hit_segment(x: f32, y: f32, count: usize) -> Option<usize> {
    if count == 0 || !(INNER_RADIUS..=OUTER_RADIUS).contains(&x.hypot(y)) {
        return None;
    }
    let step = std::f32::consts::TAU / count as f32;
    let angle =
        (y.atan2(x) + std::f32::consts::FRAC_PI_2 + step / 2.).rem_euclid(std::f32::consts::TAU);
    Some(((angle / step) as usize).min(count - 1))
}
pub fn place(cursor: (i32, i32), work: (i32, i32, i32, i32), size: (i32, i32)) -> (i32, i32) {
    (
        (cursor.0 - size.0 / 2).clamp(work.0, (work.2 - size.0).max(work.0)),
        (cursor.1 - size.1 / 2).clamp(work.1, (work.3 - size.1).max(work.1)),
    )
}
