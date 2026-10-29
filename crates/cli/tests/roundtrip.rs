//! Roundtrip tests against the local Skyrim installation:
//! Data (BSA/loose) -> FBX/KF -> HKX, compared with the original animations.
//!
//! ```sh
//! cargo test --release -p hkxc --test roundtrip -- --ignored --nocapture
//! ```
//!
//! Environment variables:
//! - `SKYRIM_DATA`: Skyrim `Data` directory (default: auto-detected by hkxc)
//! - `HKXC_TEST_ACTORS`: comma separated actors (default: `cow`)
//! - `HKXC_TEST_VERIFY_LIMIT`: animations compared per actor (default: 20, 0 = all)
//!
//! Output: `<target>/tmp/roundtrip/` (`<actor>/*.fbx` can be opened in Blender).
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::suboptimal_flops,
    reason = "test helpers: failing loudly is the intent"
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_spline::hkx::Animation;

const MAX_TRANSLATION_ERROR: f32 = 0.01;
const MAX_ROTATION_ERROR_DEG: f32 = 1.0;

#[test]
#[ignore = "needs local files"]
fn fbx_roundtrip() {
    roundtrip(Format::Fbx);
}

#[test]
#[ignore = "needs local files"]
fn kf_roundtrip() {
    roundtrip(Format::Kf);
}

#[derive(Debug, Clone, Copy)]
enum Format {
    Fbx,
    Kf,
}

impl Format {
    const fn name(self) -> &'static str {
        match self {
            Self::Fbx => "fbx",
            Self::Kf => "kf",
        }
    }
}

fn roundtrip(format: Format) {
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join("roundtrip");
    let cache = output.join("data");
    let check = output.join("check").join(format.name());
    let actors = env_list("HKXC_TEST_ACTORS", "cow");
    let limit = std::env::var("HKXC_TEST_VERIFY_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20usize);

    let skeletons = actor_skeletons();
    let mut failures = Vec::new();
    let mut compared = 0;

    for actor in &actors {
        let skeleton_path = skeletons
            .get(actor)
            .unwrap_or_else(|| panic!("unknown actor '{actor}'"));

        // 1. Data -> FBX (also copies the used files to data/).
        hkxc(&with_data(&[
            "exportall",
            "--actor",
            actor,
            "-o",
            &s(&output),
            "-v",
            "fbx",
        ]));

        // Everything below reads the files hkxc copied to data/.
        let skeleton = cache.join(skeleton_path);
        let actor_dir = skeleton_path
            .rsplit_once("/character")
            .map_or(skeleton_path.as_str(), |(dir, _)| dir);
        let animations = cache.join(actor_dir).join("animations");
        let reference = output.join("check").join("reference").join(actor);

        // 2. Reference JSON from the original hkx.
        hkxc(&[
            "exportrig",
            "-s",
            &s(&skeleton),
            "-a",
            &s(&animations),
            "-o",
            &s(&reference),
            "-v",
            "json",
        ]);

        // 3. Input of the roundtrip.
        let input = match format {
            Format::Fbx => output.join(actor),
            Format::Kf => {
                let kf = check.join("kf").join(actor);
                hkxc(&[
                    "exportrig",
                    "-s",
                    &s(&skeleton),
                    "-a",
                    &s(&animations),
                    "-o",
                    &s(&kf),
                    "-v",
                    "kf",
                ]);
                kf
            }
        };

        // 4. -> HKX -> JSON
        let hkx = check.join("hkx").join(actor);
        let json = check.join("json").join(actor);
        hkxc(&[
            "importrig",
            "-s",
            &s(&skeleton),
            "-a",
            &s(&input),
            "-o",
            &s(&hkx),
        ]);
        hkxc(&[
            "exportrig",
            "-s",
            &s(&skeleton),
            "-a",
            &s(&hkx),
            "-o",
            &s(&json),
            "-v",
            "json",
        ]);

        // 5. Compare.
        let mut files = animation_jsons(&json);
        if limit > 0 {
            files.truncate(limit);
        }
        for file in files {
            let relative = file.strip_prefix(&json).unwrap_or(&file);
            let expected = reference.join(relative);
            if !expected.exists() {
                continue;
            }
            compared += 1;

            let diff = compare(&read_animation(&expected), &read_animation(&file));
            let ok = diff.frame_count_match
                && diff.translation <= MAX_TRANSLATION_ERROR
                && diff.rotation_deg <= MAX_ROTATION_ERROR_DEG;
            println!(
                "{} {actor}/{}: translation {:.5}, rotation {:.4} deg, frames {}",
                if ok { "PASS" } else { "FAIL" },
                relative.display(),
                diff.translation,
                diff.rotation_deg,
                if diff.frame_count_match {
                    "ok"
                } else {
                    "MISMATCH"
                },
            );
            if !ok {
                failures.push(format!("{actor}/{}", relative.display()));
            }
        }
    }

    println!("{compared} compared, {} failed", failures.len());
    assert!(compared > 0, "no animation was compared");
    assert!(
        failures.is_empty(),
        "roundtrip errors over the thresholds: {failures:#?}"
    );
}

/// Runs the hkxc binary built for this test.
fn hkxc(args: &[&str]) {
    println!(">>> hkxc {}", args.join(" "));
    let status = Command::new(env!("CARGO_BIN_EXE_hkxc"))
        .args(args)
        .arg("--quiet")
        .status()
        .expect("failed to run hkxc");
    assert!(status.success(), "hkxc {} failed: {status}", args.join(" "));
}

/// Appends `--data $SKYRIM_DATA` when set.
fn with_data<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut args = args.to_vec();
    if let Ok(data) = std::env::var("SKYRIM_DATA") {
        args.push("--data");
        args.push(Box::leak(data.into_boxed_str()));
    }
    args
}

/// `hkxc exportall --list` -> actor id -> skeleton path inside Data.
fn actor_skeletons() -> HashMap<String, String> {
    let output = Command::new(env!("CARGO_BIN_EXE_hkxc"))
        .args(with_data(&["exportall", "--list"]))
        .output()
        .expect("failed to run hkxc");
    assert!(output.status.success(), "hkxc exportall --list failed");

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            // `<id>  <n> animations  <n> meshes  <skeleton path>`
            let (id, rest) = line.split_once(char::is_whitespace)?;
            let (_, skeleton) = rest.split_once(" meshes")?;
            Some((id.to_owned(), skeleton.trim().to_owned()))
        })
        .collect()
}

fn env_list(name: &str, default: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_owned())
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

fn s(path: &Path) -> String {
    path.display().to_string()
}

/// Animation JSON files under `dir` (excluding `skeleton.json` and annotations).
fn animation_jsons(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".json")
                && name != "skeleton.json"
                && !name.ends_with(".annotations.json")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn read_animation(path: &Path) -> Animation {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    sonic_rs::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

struct Diff {
    frame_count_match: bool,
    translation: f32,
    rotation_deg: f32,
}

/// Max translation distance and rotation angle between two animations.
fn compare(expected: &Animation, actual: &Animation) -> Diff {
    let mut translation = 0.0f32;
    let mut rotation_deg = 0.0f32;

    for (a, b) in expected.frames.iter().zip(&actual.frames) {
        for (ta, tb) in a.transforms.iter().zip(&b.transforms) {
            let (p, q) = (&ta.transition, &tb.transition);
            let d = ((p.x - q.x).powi(2) + (p.y - q.y).powi(2) + (p.z - q.z).powi(2)).sqrt();
            translation = translation.max(d);

            let (r, s) = (&ta.quaternion, &tb.quaternion);
            let dot = (r.x * s.x + r.y * s.y + r.z * s.z + r.scaler * s.scaler)
                .abs()
                .min(1.0);
            rotation_deg = rotation_deg.max((2.0 * dot.acos()).to_degrees());
        }
    }

    Diff {
        frame_count_match: expected.frames.len() == actual.frames.len(),
        translation,
        rotation_deg,
    }
}
