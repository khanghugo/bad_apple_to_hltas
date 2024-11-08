use std::{
    fs::OpenOptions,
    io::{Cursor, Write},
    path::Path,
    thread,
};

use image::{
    imageops::{self, BiLevel},
    DynamicImage, GenericImageView, GrayImage,
};
use vid2img::FileSource;

use serde::{Deserialize, Serialize};

//
//
// SETTINGS HERE
//
//

// scaling image
const SCALE_FACTOR: f32 = 0.125;

// cannny parameters
const SIGMA: f32 = 1.2;
const STRONG_THRESHOLD: f32 = 0.2;
const WEAK_THRESHOLD: f32 = 0.01;

// origin
const STARTING_YAW: f32 = 90.197754;
const STARTING_PITCH: f32 = -0.022000;

const SCREEN_WIDTH: u32 = 1280;
const SCREEN_HEIGHT: u32 = 720;
const ANGLE_PER_PIXEL: f32 = 0.0625 / 2.;

const ZERO_MS_FRAMETIME: &str = "0.0000000001";
const HLTAS_FRAMETIME: &str = "0.04171"; // video is 23.97602fps

// DRAW with some wait in between
const SLOW_DRAW: bool = true;
const SLOW_WAIT: &str = "0.000001";

// Caps dot count on screen
const COUNT_DOTS: bool = false;
const MAX_DOTS: usize = 240;

// Change mode of image
const MODE: Mode = Mode::Dithering;

const VIDEO_PATH: &str = "/home/khang/apple/renai_circulation.webm";
const VIDEO_DIMENSION: (u32, u32) = (1280, 720);

// one video per frame
const SEPARATE_HLTAS: bool = true;

//
//
// DONT GO BEYOND HERE
//
//

enum Mode {
    /// Detects edges from the video
    CannyEdge,
    /// Dithers the video
    Dithering,
    /// Just black and white aka pre-processed video
    BiLevel,
}

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

fn process_frame(img: DynamicImage) -> Views {
    let mut res: Views = vec![];

    match MODE {
        Mode::CannyEdge => edge_detection(img, &mut res),
        Mode::Dithering => dithering(img, &mut res),
        Mode::BiLevel => bilevel(img, &mut res),
    }

    res
}

fn edge_detection(img: impl Into<GrayImage>, res: &mut Views) {
    let detection = edge_detection::canny(
        img,
        SIGMA,            // sigma
        STRONG_THRESHOLD, // strong threshold
        WEAK_THRESHOLD,   // weak threshold
    );

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
                    (detection.width() as u32, detection.height() as u32),
                    x as u32,
                    y as u32,
                ));

                dot_count += 1;
            }
        }
    }
}

fn dithering(img: DynamicImage, res: &mut Views) {
    let mut my_image = img.into_luma8();
    let dimensions = my_image.dimensions();

    image::imageops::dither(&mut my_image, &BiLevel);

    for x in 0..dimensions.0 {
        for y in 0..dimensions.1 {
            let pixel = my_image.get_pixel(x, y);
            if pixel.0[0] > 128 {
                res.push(image_coordinate_to_viewangles(dimensions, x, y));
            }
        }
    }
}

fn bilevel(img: DynamicImage, res: &mut Views) {
    let my_image = img.into_luma8();
    let dimensions = my_image.dimensions();

    for x in 0..dimensions.0 {
        for y in 0..dimensions.1 {
            let pixel = my_image.get_pixel(x, y);
            if pixel.0[0] > 128 {
                res.push(image_coordinate_to_viewangles(dimensions, x, y));
            }
        }
    }
}

fn image_coordinate_to_viewangles(dimensions: (u32, u32), x: u32, y: u32) -> [f32; 2] {
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
            Clear::Yes => "bxt_force_clear 1; gl_clear 1; sv_zmax 1",
            Clear::No => "bxt_force_clear 0; gl_clear 0; sv_zmax 8192",
        }
    )
}

fn hltas_delay_frame() -> String {
    format!("----------|------|------|{HLTAS_FRAMETIME}|{STARTING_YAW}|{STARTING_PITCH}|1")
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
            res += format!("----------|------|------|{}|-|-|1|", SLOW_WAIT).as_str();
            res += "\n";
        }
    }

    res += hltas_delay_frame().as_str();
    res += "\n";

    res
}

fn hltas_template(hltas: String, next_frame: Option<u32>) -> String {
    // need to have at least 2 frames in a hltas
    let mut res = format!(
        "\
version 1
hlstrafe_version 5
load_command bxt_anglespeed_cap 0; gl_clear 1; bxt_force_clear 1; sv_zmax 1;
frametime0ms {ZERO_MS_FRAMETIME}
frames
strafing vectorial
target_yaw velocity_lock

----------|------|------|{ZERO_MS_FRAMETIME}|0|-|1
{hltas}
"
    );

    if let Some(next_frame) = next_frame {
        res += format!(
            "\
----------|------|------|{ZERO_MS_FRAMETIME}|0|-|1|echo \"frame {}\"; bxt_tas_loadscript out/{}.hltas",
            next_frame,
            next_frame
        )
        .as_str();
    }

    res
}

fn main() {
    let file_path = Path::new(VIDEO_PATH);
    let dimensions = VIDEO_DIMENSION;

    let frame_source = FileSource::new(file_path, dimensions).unwrap();

    let my_iter = frame_source.into_iter();
    let iter_again = my_iter.enumerate();

    let mut hltas_res = String::new();

    let separate_out_folder = file_path.with_file_name("out");

    if SEPARATE_HLTAS {
        match std::fs::create_dir(separate_out_folder.as_path()) {
            Ok(_) => (),
            Err(err) => match err.kind() {
                std::io::ErrorKind::AlreadyExists => (),
                _ => panic!("cannot create `out` dir for hltas: {}", err),
            },
        };
    }

    // video conversion
    let mut count = 0;
    let max = 1500;
    for (index, frame) in iter_again {
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

            if count >= max {
                break;
            }

            if SEPARATE_HLTAS {
                let local_count = count;
                let local_separtate_folder = separate_out_folder.clone();

                let _handle = thread::spawn(move || {
                    let res = hltas_template(hltas_frame_res, Some(local_count as u32 + 1));
                    let mut file = OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(
                            local_separtate_folder
                                .join(local_count.to_string())
                                .with_extension("hltas"),
                        )
                        .expect("cannot create new hltas file in `out` folder");

                    write!(file, "{}", res).expect("cannot write to new hltas file");
                    file.flush().expect("cannot flush new hltas file");
                });
            } else {
                hltas_res += hltas_frame_res.as_str();
            }

            count += 1;
        }
    }

    // single image conversion
    // let image = image::open("/home/khang/apple/xdd.png").unwrap();
    // let image = resize_image(image);
    // let res = process_frame(image);
    // let hltas_res = frame_views_to_hltas(res);

    if !SEPARATE_HLTAS {
        let res = hltas_template(hltas_res, None);

        println!("{res}");
    }
}
