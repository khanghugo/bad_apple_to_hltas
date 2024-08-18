use std::{io::Cursor, path::Path};

use image::{imageops, DynamicImage, GenericImageView, GrayImage};
use vid2img::FileSource;

use serde::{Deserialize, Serialize};

// scaling image
static SCALE_FACTOR: f32 = 1.;

// cannny parameters
static SIGMA: f32 = 1.2;
static STRONG_THRESHOLD: f32 = 0.2;
static WEAK_THRESHOLD: f32 = 0.01;

// origin
static STARTING_YAW: f32 = 90.109863;
static STARTING_PITCH: f32 = 0.061642;

static SCREEN_WIDTH: u32 = 1024;
static SCREEN_HEIGHT: u32 = 768;
static ANGLE_PER_PIXEL: f32 = 0.0625;

static ZERO_MS_FRAMETIME: &str = "0.0000000001";
static HLTAS_FRAMETIME: &str = "0.03333333333333333"; // video is 30fps

static SLOW_DRAW: bool = true;
static COUNT_DOTS: bool = true;

type Views = Vec<[f32; 2]>;

#[derive(Debug, Serialize, Deserialize)]
struct Frame {
    viewangles: Vec<[f32; 2]>,
}

fn resize_image(img: DynamicImage) -> DynamicImage {
    let dimensions = img.dimensions();
    img.resize(
        (dimensions.0 as f32 * SCALE_FACTOR) as u32,
        (dimensions.1 as f32 * SCALE_FACTOR) as u32,
        imageops::FilterType::Nearest,
    )
}

fn process_frame(img: impl Into<GrayImage>) -> Views {
    let detection = edge_detection::canny(
        img,
        SIGMA,            // sigma
        STRONG_THRESHOLD, // strong threshold
        WEAK_THRESHOLD,   // weak threshold
    );

    let mut res: Views = vec![];
    const MAX_DOTS: usize = 240;
    let mut dot_count = 0;

    for x in 0..detection.width() {
        for y in 0..detection.height() {
            let edge = detection.interpolate(x as f32, y as f32);
            let magnitude = edge.magnitude();

            if dot_count >= MAX_DOTS && COUNT_DOTS {
                break;
            }

            if magnitude > 0. {
                res.push(image_coordinate_to_viewangles(
                    (detection.width(), detection.height()),
                    x,
                    y,
                ));

                dot_count += 1;
            }
        }
    }

    res
}

fn image_coordinate_to_viewangles(dimensions: (usize, usize), x: usize, y: usize) -> [f32; 2] {
    let center_x = dimensions.0 / 2;
    let center_y = dimensions.1 / 2;

    let diff_x = x as i32 - center_x as i32;
    let diff_y = y as i32 - center_y as i32;

    // pitch is y
    // flip the pitch
    let pitch = STARTING_PITCH
        - diff_y as f32 / dimensions.1 as f32 * SCREEN_HEIGHT as f32 * ANGLE_PER_PIXEL;
    let yaw =
        diff_x as f32 / dimensions.0 as f32 * SCREEN_WIDTH as f32 * ANGLE_PER_PIXEL + STARTING_YAW;

    [pitch, yaw]
}

enum Clear {
    None,
    Yes,
    No,
}

fn hltas_change_view_frame(pitch: f32, yaw: f32, should_clear: Clear) -> String {
    format!(
        "----------|------|------|{ZERO_MS_FRAMETIME}|{}|{}|1|{}",
        yaw,
        pitch,
        match should_clear {
            Clear::None => "",
            Clear::Yes => "bxt_force_clear 1",
            Clear::No => "bxt_force_clear 0",
        }
    )
}

fn hltas_delay_frame() -> String {
    format!("----------|------|------|{HLTAS_FRAMETIME}|-|-|1")
}

fn frame_views_to_hltas(views: Views) -> String {
    if views.is_empty() {
        return "".to_string();
    }

    let mut res = String::new();

    for (idx, view) in views.iter().enumerate() {
        res += hltas_change_view_frame(
            view[0],
            view[1],
            if idx == 0 {
                Clear::Yes
            } else if idx == 1 {
                Clear::No
            } else {
                Clear::None
            },
        )
        .as_str();
        res += "\n";

        if SLOW_DRAW {
            res += "----------|------|------|0.001|-|-|1|";
            res += "\n";
        }
    }

    res += hltas_delay_frame().as_str();
    res += "\n";

    res
}

fn main() {
    let file_path = Path::new("/home/khang/apple/badapple.webm");
    let dimensions = (480, 360);

    let frame_source = FileSource::new(file_path, dimensions).unwrap();

    let my_iter = frame_source.into_iter();
    let iter_again = my_iter.enumerate();

    let mut hltas_res = String::new();

    for (_index, frame) in iter_again {
        if let Ok(Some(png_img_data)) = frame {
            let cursor = Cursor::new(png_img_data);
            let image = image::io::Reader::new(cursor)
                .with_guessed_format()
                .unwrap()
                .decode()
                .unwrap();

            let image = resize_image(image);
            let frame_res = process_frame(image);
            let hltas_frame_res = frame_views_to_hltas(frame_res);
            hltas_res += hltas_frame_res.as_str();
        }
    }

    // single image conversion
    // let image = image::open("/home/khang/apple/dreamybull2.png").unwrap();
    // let image = resize_image(image);
    // let res = process_frame(image, 0);
    // let res = frame_views_to_hltas(res);

    let res = format!(
        "\
version 1
hlstrafe_version 5
load_command bxt_anglespeed_cap 0;
frametime0ms 0.0000000001
frames
strafing vectorial
target_yaw velocity_lock

{hltas_res}
"
    );

    println!("{res}");
}
