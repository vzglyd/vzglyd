use crate::date_utils;

use std::time::{SystemTime, UNIX_EPOCH};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use vzglyd_slide::{CameraPath, WorldVertex as Vertex};

use crate::render_context::{HEIGHT, WIDTH};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct WorldUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub cam_pos: [f32; 3],
    pub time: f32,
    pub fog_color: [f32; 4],
    pub fog_start: f32,
    pub fog_end: f32,
    pub clock_seconds: f32,
    pub _pad: f32,
    pub ambient_light: [f32; 4],
    pub main_light_dir: [f32; 4],
    pub main_light_color: [f32; 4],
}

const CHAR_ORDER: &[u8] = b" ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:";
const PX_W: f32 = 2.0 / WIDTH as f32 * 2.0;
const PX_H: f32 = 2.0 / HEIGHT as f32 * 2.0;
const MELBOURNE_STD_OFFSET_SECS: i32 = 10 * 60 * 60;
const MELBOURNE_DST_OFFSET_SECS: i32 = 11 * 60 * 60;

pub fn melbourne_clock_seconds() -> f32 {
    let (_, _, _, hour, minute, second) = epoch_to_melbourne_components(now_unix_secs());
    f32::from(hour) * 3_600.0 + f32::from(minute) * 60.0 + f32::from(second)
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn epoch_to_melbourne_components(epoch_secs: u64) -> (i32, u8, u8, u8, u8, u8) {
    let mut offset = MELBOURNE_STD_OFFSET_SECS;
    for _ in 0..2 {
        let shifted = (epoch_secs as i64 + i64::from(offset)) as u64;
        let (year, month, day, hour, minute, second) = date_utils::utc_ymdhms_from_unix(shifted);
        let next_offset = melbourne_offset_seconds(year, month, day, hour);
        if next_offset == offset {
            return (year, month, day, hour, minute, second);
        }
        offset = next_offset;
    }
    date_utils::utc_ymdhms_from_unix((epoch_secs as i64 + i64::from(offset)) as u64)
}

fn melbourne_offset_seconds(year: i32, month: u8, day: u8, hour: u8) -> i32 {
    let first_sunday_october = first_sunday(year, 10);
    let first_sunday_april = first_sunday(year, 4);
    let is_dst = if !(4..10).contains(&month) {
        true
    } else if (5..=9).contains(&month) {
        false
    } else if month == 10 {
        day > first_sunday_october || (day == first_sunday_october && hour >= 2)
    } else {
        day < first_sunday_april || (day == first_sunday_april && hour < 3)
    };
    if is_dst {
        MELBOURNE_DST_OFFSET_SECS
    } else {
        MELBOURNE_STD_OFFSET_SECS
    }
}

fn first_sunday(year: i32, month: u8) -> u8 {
    (1..=7)
        .find(|day| date_utils::weekday_abbrev(year, month, *day) == "Sun")
        .unwrap_or(1)
}

#[cfg(test)]
fn glyph(c: u8) -> [u8; 7] {
    match c {
        b' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        b'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        b'E' => [0x1F, 0x10, 0x10, 0x1C, 0x10, 0x10, 0x1F],
        b'F' => [0x1F, 0x10, 0x10, 0x1C, 0x10, 0x10, 0x10],
        b'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0E],
        b'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'I' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1F],
        b'J' => [0x0F, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        b'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        b'M' => [0x11, 0x1B, 0x15, 0x11, 0x11, 0x11, 0x11],
        b'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        b'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        b'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x13, 0x0F],
        b'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        b'S' => [0x0E, 0x11, 0x10, 0x0E, 0x01, 0x11, 0x0E],
        b'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'V' => [0x11, 0x11, 0x11, 0x11, 0x0A, 0x0A, 0x04],
        b'W' => [0x11, 0x11, 0x11, 0x15, 0x1B, 0x11, 0x11],
        b'X' => [0x11, 0x0A, 0x04, 0x04, 0x04, 0x0A, 0x11],
        b'Y' => [0x11, 0x0A, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        b'3' => [0x0E, 0x11, 0x01, 0x06, 0x01, 0x11, 0x0E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        b'6' => [0x0E, 0x10, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        b'7' => [0x1F, 0x01, 0x02, 0x04, 0x04, 0x04, 0x04],
        b'8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        b'9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x11, 0x0E],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        _ => [0x00; 7],
    }
}

fn char_uv(c: u8) -> (f32, f32, f32, f32) {
    let c = c.to_ascii_uppercase();
    if let Some(ci) = CHAR_ORDER.iter().position(|&x| x == c) {
        let x0 = (ci * 6) as f32 / 256.0;
        let x1 = (ci * 6 + 5) as f32 / 256.0;
        (x0, 0.0, x1, 7.0 / 8.0)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    }
}

fn push_text(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u16>,
    text: &[u8],
    x: f32,
    y: f32,
    color: [f32; 4],
    mode: f32,
) {
    let cw = 6.0 * PX_W;
    let ch = 7.0 * PX_H;
    for (i, &c) in text.iter().enumerate() {
        let cx = x + i as f32 * cw;
        let cy = y - ch;
        let (u0, v0, u1, v1) = char_uv(c);
        let b = verts.len() as u16;
        for (pos, uv) in [
            ([cx, cy, 0.0_f32], [u0, v1, 0.0_f32]),
            ([cx + cw, cy, 0.0], [u1, v1, 0.0]),
            ([cx + cw, cy + ch, 0.0], [u1, v0, 0.0]),
            ([cx, cy + ch, 0.0], [u0, v0, 0.0]),
        ] {
            verts.push(Vertex {
                position: pos,
                normal: uv,
                color,
                mode,
            });
        }
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
}

#[allow(dead_code)]
pub fn build_fps_text(verts: &mut Vec<Vertex>, indices: &mut Vec<u16>, fps: u32) {
    build_fps_text_with_mode(verts, indices, fps, 4.0);
}

pub fn build_fps_text_with_mode(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u16>,
    fps: u32,
    mode: f32,
) {
    let s = format!("FPS:{:3}", fps.min(999));
    let bytes = s.as_bytes();
    let cw = 6.0 * PX_W;
    let x = 1.0 - bytes.len() as f32 * cw - 0.01;
    let y = 0.97;
    push_text(verts, indices, bytes, x, y, [1.0, 0.92, 0.20, 1.0], mode);
}


fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

pub fn sample_camera(
    path: &Option<CameraPath>,
    elapsed: f32,
    fallback_cx: f32,
    fallback_cz: f32,
) -> (Vec3, Vec3, Vec3, f32) {
    if let Some(p) = path {
        if p.keyframes.len() >= 2 {
            let duration = p.keyframes.last().expect("camera path not empty").time;
            let t = if duration > 0.0 && p.looped {
                elapsed % duration
            } else {
                elapsed.min(duration)
            };
            let mut i = 0;
            while i + 1 < p.keyframes.len() && p.keyframes[i + 1].time < t {
                i += 1;
            }
            let a = &p.keyframes[i];
            let b = &p.keyframes[(i + 1).min(p.keyframes.len() - 1)];
            let span = (b.time - a.time).max(0.0001);
            let lt = ((t - a.time) / span).clamp(0.0, 1.0);
            let eye = Vec3::new(
                lerp(a.position[0], b.position[0], lt),
                lerp(a.position[1], b.position[1], lt),
                lerp(a.position[2], b.position[2], lt),
            );
            let target = Vec3::new(
                lerp(a.target[0], b.target[0], lt),
                lerp(a.target[1], b.target[1], lt),
                lerp(a.target[2], b.target[2], lt),
            );
            let up = Vec3::new(
                lerp(a.up[0], b.up[0], lt),
                lerp(a.up[1], b.up[1], lt),
                lerp(a.up[2], b.up[2], lt),
            )
            .normalize_or_zero();
            let fov = lerp(a.fov_y_deg, b.fov_y_deg, lt);
            return (eye, target, up, fov);
        }
    }

    let t = smoothstep((elapsed % 24.0) / 24.0);
    let ex = lerp(14.0, -3.0, t);
    let ez = lerp(50.0, 24.0, t);
    let eye_y = 4.5;
    let eye = Vec3::new(ex, eye_y, ez);
    let target_y = 2.5;
    let target = Vec3::new(fallback_cx, target_y, fallback_cz);
    (eye, target, Vec3::Y, 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uniforms_size() {
        assert_eq!(std::mem::size_of::<WorldUniforms>(), 160);
    }

    #[test]
    fn font_atlas_size() {
        assert_eq!(vzglyd_slide::make_font_atlas().len(), 256 * 8 * 4);
    }

    #[test]
    fn glyph_known_chars_nonzero() {
        for &c in b"VZGLYD TERRAIN BENCHMARK TARGET RASPBERRY PI 4".iter() {
            if c != b' ' {
                assert!(
                    glyph(c).iter().any(|&b| b != 0),
                    "empty glyph for '{}'",
                    c as char
                );
            }
        }
    }

    #[test]
    fn fps_text_produces_quads() {
        let mut v = Vec::new();
        let mut i = Vec::new();
        build_fps_text(&mut v, &mut i, 60);
        assert_eq!(v.len(), 7 * 4);
        assert_eq!(i.len(), 7 * 6);
    }
}
