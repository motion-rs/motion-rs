//! Test-only offscreen rasterizer that renders the studio workbench to PNG snapshots.

use super::*;
use motion_core::prelude::*;
use motion_render::Composition;
use std::collections::HashMap;
use std::time::Duration as StdDuration;

fn color_of(data: &egui::ImageData) -> egui::ColorImage {
    match data {
        egui::ImageData::Color(image) => (**image).clone(),
        egui::ImageData::Font(font) => egui::ColorImage {
            size: font.size,
            pixels: font.srgba_pixels(None).collect(),
        },
    }
}

fn rasterize(
    output: &egui::FullOutput,
    textures: &mut HashMap<egui::TextureId, egui::ColorImage>,
    context: &egui::Context,
    width: usize,
    height: usize,
    scale: f32,
) -> Vec<[f32; 4]> {
    for (id, delta) in &output.textures_delta.set {
        let patch = color_of(&delta.image);
        match delta.pos {
            None => {
                textures.insert(*id, patch);
            }
            Some([x, y]) => {
                let target = textures.get_mut(id).expect("patched texture");
                for row in 0..patch.size[1] {
                    for column in 0..patch.size[0] {
                        target.pixels[(y + row) * target.size[0] + x + column] =
                            patch.pixels[row * patch.size[0] + column];
                    }
                }
            }
        }
    }
    let mut frame = vec![[0.1, 0.1, 0.1, 1.0]; width * height];
    for primitive in context.tessellate(output.shapes.clone(), scale) {
        let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
            continue;
        };
        let Some(texture) = textures.get(&mesh.texture_id) else {
            continue;
        };
        let clip = primitive.clip_rect;
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let v = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            let p = v.map(|vertex| (vertex.pos.x * scale, vertex.pos.y * scale));
            let area =
                (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
            if area.abs() < 1e-6 {
                continue;
            }
            let min_x = p
                .iter()
                .map(|p| p.0)
                .fold(f32::MAX, f32::min)
                .max(clip.left() * scale)
                .floor()
                .max(0.0) as usize;
            let max_x = p
                .iter()
                .map(|p| p.0)
                .fold(f32::MIN, f32::max)
                .min(clip.right() * scale)
                .ceil()
                .min(width as f32) as usize;
            let min_y = p
                .iter()
                .map(|p| p.1)
                .fold(f32::MAX, f32::min)
                .max(clip.top() * scale)
                .floor()
                .max(0.0) as usize;
            let max_y = p
                .iter()
                .map(|p| p.1)
                .fold(f32::MIN, f32::max)
                .min(clip.bottom() * scale)
                .ceil()
                .min(height as f32) as usize;
            for y in min_y..max_y {
                for x in min_x..max_x {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p[1].0 - px) * (p[2].1 - py) - (p[2].0 - px) * (p[1].1 - py)) / area;
                    let w1 = ((p[2].0 - px) * (p[0].1 - py) - (p[0].0 - px) * (p[2].1 - py)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                    let t = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                    let tx = u.rem_euclid(1.0) * texture.size[0] as f32;
                    let ty = t.rem_euclid(1.0) * texture.size[1] as f32;
                    let sample = texture.pixels[(ty as usize).min(texture.size[1] - 1)
                        * texture.size[0]
                        + (tx as usize).min(texture.size[0] - 1)];
                    let mix = |a: u8, b: u8, c: u8, d: f32, e: f32, f: f32| {
                        (f32::from(a) * d + f32::from(b) * e + f32::from(c) * f) / 255.0
                    };
                    let channel = |pick: fn(egui::Color32) -> u8| -> f32 {
                        mix(
                            pick(v[0].color),
                            pick(v[1].color),
                            pick(v[2].color),
                            w0,
                            w1,
                            w2,
                        ) * f32::from(pick(sample))
                            / 255.0
                    };
                    let src = [
                        channel(|c| c.r()),
                        channel(|c| c.g()),
                        channel(|c| c.b()),
                        channel(|c| c.a()),
                    ];
                    let dst = &mut frame[y * width + x];
                    for index in 0..3 {
                        dst[index] = src[index] + dst[index] * (1.0 - src[3]);
                    }
                }
            }
        }
    }
    frame
}

#[test]
#[ignore]
fn dump_workbench_png() -> motion_core::Result<()> {
    use eframe::App;
    let out = std::env::var("STUDIO_SNAPSHOT_DIR").expect("STUDIO_SNAPSHOT_DIR");
    let scale = 1.0f32;
    for (name, dark, tab, width, height) in [
        ("dark", true, 0usize, 1440usize, 900usize),
        ("light", false, 0, 1440, 900),
        ("dark-stats", true, 3, 1440, 900),
        ("narrow", true, 0, 900, 620),
    ] {
        let context = egui::Context::default();
        let creation = eframe::CreationContext::_new_kittest(context.clone());
        let timeline = motion_core::Timeline::new(Size::new(960.0, 540.0), 30)
            .track(|track| {
                track
                    .scene(3.seconds(), |scene| {
                        let x = 100.0 + scene.progress() * 600.0;
                        Node::group((
                            Rect::new(0.0, 0.0, 960.0, 540.0).fill(Color::rgb(24, 60, 90)),
                            Circle::new(Point::new(x, 270.0), 80.0).fill(Color::rgb(240, 180, 60)),
                        ))
                    })
                    .transition(CrossFade, 500.milliseconds())
                    .scene(3.seconds(), |scene| {
                        Node::group((
                            Rect::new(0.0, 0.0, 960.0, 540.0).fill(Color::rgb(90, 30, 60)),
                            Circle::new(scene.center(), 40.0 + 120.0 * scene.progress())
                                .fill(Color::WHITE),
                        ))
                    })
            })
            .track(|track| {
                track.gap(1.seconds()).scene(2.seconds(), |_| {
                    Rect::new(60.0, 60.0, 300.0, 120.0).fill(Color::rgba(255, 255, 255, 160))
                })
            });
        let scene = StudioScene::timeline("Demo", Composition::new(960, 540), timeline)?;
        let clock = StudioClock::new(&scene.composition)?;
        let mut app = StudioApp::with_persistence(&creation, scene, clock, false)?;
        crate::theme::apply(&context, dark);
        app.observations.backend_status = "CPU".into();
        app.inspection.inspector_tab = tab;
        app.inspection.output_visible = Some(true);
        app.inspection.viewer.show_safe_area = true;
        app.seek(40);
        app.exports.export_range = (10, 150);
        let mut frame = eframe::Frame::_new_kittest();
        let mut textures = HashMap::new();
        let mut last = None;
        let deadline = Instant::now() + StdDuration::from_secs(20);
        while Instant::now() < deadline {
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width as f32 / scale, height as f32 / scale),
                    )),
                    ..Default::default()
                },
                |context| app.update(context, &mut frame),
            );
            let ready = app.texture.is_some();
            last = Some(rasterize(
                &output,
                &mut textures,
                &context,
                width,
                height,
                scale,
            ));
            // Keep applying deltas for later passes
            if ready {
                std::thread::sleep(StdDuration::from_millis(200));
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width as f32 / scale, height as f32 / scale),
                        )),
                        ..Default::default()
                    },
                    |context| app.update(context, &mut frame),
                );
                last = Some(rasterize(
                    &output,
                    &mut textures,
                    &context,
                    width,
                    height,
                    scale,
                ));
                break;
            }
            std::thread::sleep(StdDuration::from_millis(50));
        }
        let pixels = last.expect("frame");
        let mut image = image::RgbaImage::new(width as u32, height as u32);
        for (index, pixel) in pixels.iter().enumerate() {
            image.put_pixel(
                (index % width) as u32,
                (index / width) as u32,
                image::Rgba([
                    (pixel[0].clamp(0.0, 1.0) * 255.0) as u8,
                    (pixel[1].clamp(0.0, 1.0) * 255.0) as u8,
                    (pixel[2].clamp(0.0, 1.0) * 255.0) as u8,
                    255,
                ]),
            );
        }
        image.save(format!("{out}/{name}.png")).expect("save");
    }
    Ok(())
}
