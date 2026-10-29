//! Export every actor animation of a Skyrim `Data` directory.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_hkx_features::error::Error;
use serde_spline::hkx::Skeleton;

use crate::{AnyError, ui};

#[cfg(feature = "fbx")]
use super::actor::{
    NifSource, TextureMode, externalize_textures, filter_nifs, load_meshes, relative_textures,
};
use super::actor::{
    Actor, EXPECTED_TREE, EXTRACT_DIR_NAME, RigArgs, RigResolver, discover_actors, find_actor,
};
use super::export::{Format, write_animation};

const EXAMPLES: &str = r#"Examples

- <blue!>Export every vanilla animation to FBX (Data directory found automatically)</blue!>
  <cyan!>hkxc exportall -o</cyan!> ./out

- <blue!>Export some actors only</blue!>
  <cyan!>hkxc exportall --actor</cyan!> cow <cyan!>--actor</cyan!> canine/wolf <cyan!>-o</cyan!> ./out

- <blue!>List the actors without exporting</blue!>
  <cyan!>hkxc exportall --list</cyan!>

- <blue!>Use a specific Data directory (loose files override BSA archives)</blue!>
  <cyan!>hkxc exportall --data</cyan!> "D:/Steam/steamapps/common/Skyrim Special Edition/Data" <cyan!>-o</cyan!> ./out

Output:
  <<OUTPUT>>/<<actor>>/<<path under animations/>>.fbx   e.g. out/cow/getup.fbx
  <<OUTPUT>>/data/...   files used from the Data directory (skeleton, nif, hkx, dds),
                      in the Data layout. Later runs read them from here first,
                      and work without Skyrim installed (or use --data <<OUTPUT>>/data).
"#;

#[derive(Debug, clap::Args)]
#[clap(after_long_help = crate::ui::paint(&format!("{EXAMPLES}\n{EXPECTED_TREE}")))]
pub(crate) struct Args {
    /// Skyrim `Data` directory (loose files + BSA archives).
    ///
    /// Defaults to the Skyrim Special Edition installation found through the
    /// registry or Steam libraries.
    #[clap(long, value_name = "DATA_DIR")]
    pub data: Option<PathBuf>,

    /// Actors to export (default: all), e.g. `cow`, `canine/wolf`.
    #[clap(long, value_name = "ACTOR")]
    pub actor: Vec<String>,

    /// Only list the actors found and their animation counts.
    #[clap(long)]
    pub list: bool,

    /// Output directory.
    #[clap(short, long, value_name = "OUTPUT", default_value = "output")]
    pub output: PathBuf,

    /// Output format.
    #[clap(short = 'v', long, value_enum, value_name = "FORMAT", default_value = "fbx")]
    pub format: Format,

    /// Scene frame rate written to FBX (default: the frame rate of each animation).
    #[cfg(feature = "fbx")]
    #[clap(long)]
    pub fps: Option<f32>,

    /// Do not add the actors' meshes to FBX.
    #[cfg(feature = "fbx")]
    #[clap(long)]
    pub no_mesh: bool,

    /// Only use NIFs whose file name contains this text (repeatable).
    #[cfg(feature = "fbx")]
    #[clap(long, value_name = "NAME")]
    pub mesh: Vec<String>,

    /// How FBX meshes get their textures. `external` writes each DDS once to
    /// `<OUTPUT>/textures/` instead of copying it into every FBX.
    #[cfg(feature = "fbx")]
    #[clap(long, value_enum, default_value = "external")]
    pub textures: TextureMode,
}

/// Exports the animations of every (or the selected) actor.
///
/// Failures of single animations are reported and skipped.
///
/// # Errors
/// If the Data directory cannot be opened or a requested actor does not exist.
pub fn export_all(args: &Args) -> Result<(), AnyError> {
    let rig_args = RigArgs {
        actor: None,
        auto: true, // only to satisfy the resolver; actors are passed explicitly
        data: args.data.clone(),
    };
    let extract_dir = args.output.join(EXTRACT_DIR_NAME);
    let resolver = RigResolver::new(&rig_args, None)?.with_extract_dir(extract_dir.clone());

    let spinner = ui::spinner("Opening the Data directory (BSA indexes)...");
    let data = resolver.data()?;
    spinner.set_message("Finding actors...");
    let actors = discover_actors(&data);
    spinner.finish_and_clear();

    let selected: Vec<_> = if args.actor.is_empty() {
        actors.iter().collect()
    } else {
        args.actor
            .iter()
            .map(|query| find_actor(&actors, query))
            .collect::<Result<_, _>>()?
    };

    if args.list {
        for actor in &selected {
            println!(
                "{:<32} {:>4} animations  {:>2} meshes  {}",
                actor.id,
                actor.animations(&data).len(),
                actor.nifs.len(),
                actor.skeleton
            );
        }
        return Ok(());
    }

    // 1. Skeletons, meshes and textures of every actor, in parallel.
    let preparing = ui::progress_bar(selected.len() as u64, "Preparing");
    let prepared: Vec<Result<ActorJob, String>> = selected
        .par_iter()
        .map(|actor| {
            preparing.set_message(actor.id.clone());
            let job = prepare_actor(args, &resolver, &data, actor, &extract_dir);
            preparing.inc(1);
            job
        })
        .collect();
    preparing.finish_and_clear();

    let mut failed: Vec<String> = Vec::new();
    let jobs: Vec<ActorJob> = prepared
        .into_iter()
        .filter_map(|job| job.map_err(|error| failed.push(error)).ok())
        .collect();

    // 2. Every animation of every actor, in parallel.
    let total = jobs.iter().map(|job| job.animations.len()).sum::<usize>();
    let exporting = ui::progress_bar(total as u64, "Exporting");
    let results: Vec<Result<(), String>> = jobs
        .par_iter()
        .flat_map(|job| job.animations.par_iter().map(move |animation| (job, animation)))
        .map(|(job, (path, relative))| {
            exporting.set_message(format!("{}/{relative}", job.id));
            let result = export_one(args, &resolver, &extract_dir, job, path, relative);
            exporting.inc(1);
            result
        })
        .collect();
    exporting.finish_and_clear();

    let exported = results.iter().filter(|r| r.is_ok()).count();
    failed.extend(results.into_iter().filter_map(Result::err));

    for failure in &failed {
        tracing::warn!("failed: {failure}");
    }
    ui::info(&format!(
        "<green>Exported {exported} animations</green> to '{}' ({} failed{})",
        ui::escape(&args.output.display().to_string()),
        failed.len(),
        if failed.is_empty() { "" } else { ", see the log with --stdout --log-level warn" }
    ));

    if exported == 0 && !failed.is_empty() {
        return Err(Error::IoError {
            source: std::io::Error::other("no animation could be exported"),
        }
        .into());
    }
    Ok(())
}

/// One actor ready to export.
struct ActorJob {
    id: String,
    skeleton: Skeleton,
    #[cfg(feature = "fbx")]
    meshes: Vec<serde_fbx::ser::Mesh>,
    /// `(path in Data, path relative to animations/)`
    animations: Vec<(String, String)>,
}

/// Reads an actor's skeleton and meshes (copying textures when external).
fn prepare_actor(
    args: &Args,
    resolver: &RigResolver,
    data: &serde_bsa::DataDir,
    actor: &Actor,
    #[cfg_attr(not(feature = "fbx"), expect(unused_variables))] extract_dir: &Path,
) -> Result<ActorJob, String> {
    let bytes = resolver.read_data(&actor.skeleton).map_err(|e| e.to_string())?;
    let skeleton = Skeleton::from_bytes(&bytes, Path::new(&actor.skeleton))
        .map_err(|error| format!("{}: {error}", actor.skeleton))?;

    #[cfg(feature = "fbx")]
    let meshes = {
        let mut meshes: Vec<_> = if args.no_mesh || !matches!(args.format, Format::Fbx | Format::FbxAscii) {
            Vec::new()
        } else {
            filter_nifs(&actor.nifs, &args.mesh)
                .iter()
                .filter_map(|nif| {
                    resolver
                        .read_data(nif)
                        .map_err(|error| tracing::warn!("skipping mesh {nif}: {error}"))
                        .ok()?;
                    load_meshes(NifSource::Data(nif), Some(data), args.textures != TextureMode::None)
                        .map_err(|error| tracing::warn!("skipping mesh {nif}: {error}"))
                        .ok()
                })
                .flatten()
                .collect()
        };
        if args.textures == TextureMode::External {
            externalize_textures(&mut meshes, extract_dir).map_err(|e| e.to_string())?;
        }
        meshes
    };
    #[cfg(not(feature = "fbx"))]
    let _ = (args, data);

    Ok(ActorJob {
        id: actor.id.clone(),
        skeleton,
        #[cfg(feature = "fbx")]
        meshes,
        animations: actor.animations(data),
    })
}

/// Converts one animation of `job`.
fn export_one(
    args: &Args,
    resolver: &RigResolver,
    #[cfg_attr(not(feature = "fbx"), expect(unused_variables))] extract_dir: &Path,
    job: &ActorJob,
    path: &str,
    relative: &str,
) -> Result<(), String> {
    let bytes = resolver.read_data(path).map_err(|e| e.to_string())?;
    let mut output = args.output.join(&job.id).join(relative);
    output.set_extension(args.format.extension());

    #[cfg(feature = "fbx")]
    let relative_meshes = (args.textures == TextureMode::External)
        .then(|| relative_textures(&job.meshes, &output, extract_dir));
    #[cfg(feature = "fbx")]
    let meshes = relative_meshes.as_deref().unwrap_or(&job.meshes);

    write_animation(
        &bytes,
        Path::new(path),
        &job.skeleton,
        &output,
        args.format,
        #[cfg(feature = "fbx")]
        args.fps,
        #[cfg(feature = "fbx")]
        meshes,
    )
    .map_err(|e| format!("{path}: {e}"))
}
