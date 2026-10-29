//! Resolve skeletons, meshes and textures from a Skyrim `Data` directory.
//!
//! The `Data` directory (loose files + every `*.bsa`) is read as one virtual
//! file system, so vanilla assets work without extracting archives.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use serde_bsa::DataDir;
use serde_hkx_features::error::Error;
use serde_spline::hkx::Skeleton;

use super::invalid_input;

/// Directory layout expected by `--actor` / `--auto`, shown in `--help`.
pub const EXPECTED_TREE: &str = r#"Expected layout (loose files or inside BSA archives):
  Data/
  └─ meshes/actors/<actor>/          e.g. cow, canine, dlc01/chaurusflyer
     ├─ character assets*/           "character assets", "character assets wolf", ...
     │  ├─ skeleton*.hkx             -> skeleton (required)
     │  └─ *.nif                     -> meshes added to FBX (skeleton*.nif excluded)
     ├─ *.nif                        -> meshes added to FBX
     └─ animations/**/*.hkx          -> animations
  Data/textures/**/*.dds             -> textures referenced by the meshes (see --textures)

Actor names: the path under meshes/actors, plus the suffix of
"character assets <suffix>" when an actor has several skeletons:
  cow, bear, canine/dog, canine/wolf, character, character/female, dlc01/chaurusflyer

--auto: the actor is inferred from each animation path
  .../meshes/actors/<actor>/animations/.../<animation>
  A skeleton in the same mod tree (<actor>/character assets*/skeleton*.hkx)
  is preferred over the one in the Data directory."#;

/// `--actor`, `--auto` and `--data`, shared by the rig commands.
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct RigArgs {
    /// Actor whose skeleton (and meshes) are taken from the Data directory,
    /// e.g. `cow`, `canine/wolf`. See the layout below.
    #[clap(long, value_name = "ACTOR", conflicts_with = "auto")]
    pub actor: Option<String>,

    /// Infer the actor from each animation path
    /// (`.../meshes/actors/<actor>/animations/...`).
    #[clap(long)]
    pub auto: bool,

    /// Skyrim `Data` directory (loose files + BSA archives).
    ///
    /// Defaults to the Skyrim Special Edition installation found through the
    /// registry or Steam libraries.
    #[clap(long, value_name = "DATA_DIR")]
    pub data: Option<PathBuf>,
}

/// A skeleton found in the Data directory.
#[derive(Debug, Clone)]
pub struct Actor {
    /// e.g. `cow`, `canine/wolf`
    pub id: String,
    /// e.g. `meshes/actors/canine`
    pub dir: String,
    /// e.g. `meshes/actors/canine/character assets wolf/skeleton.hkx`
    pub skeleton: String,
    pub nifs: Vec<String>,
}

impl Actor {
    /// Animation files of this actor, `(path, path relative to animations/)`.
    pub fn animations(&self, data: &DataDir) -> Vec<(String, String)> {
        let prefix = format!("{}/animations/", self.dir);
        data.paths_under(&prefix)
            .into_iter()
            .filter(|p| p.ends_with(".hkx"))
            .map(|p| {
                let relative = p[prefix.len()..].to_owned();
                (p, relative)
            })
            .collect()
    }
}

/// Finds every actor skeleton under `meshes/actors/`.
pub fn discover_actors(data: &DataDir) -> Vec<Actor> {
    let paths = data.paths_under("meshes/actors/");

    let mut actors: Vec<Actor> = paths
        .iter()
        .filter_map(|path| {
            let (folder, file) = path.rsplit_once('/')?;
            let (dir, assets) = folder.rsplit_once('/')?;
            let is_skeleton = file.starts_with("skeleton") && file.ends_with(".hkx");
            let is_assets = assets.starts_with("character assets") || assets == "characterassets";
            if !is_skeleton || !is_assets {
                return None;
            }

            let base = dir.strip_prefix("meshes/actors/")?;
            let suffix = assets
                .trim_start_matches("character assets")
                .trim_start_matches("characterassets")
                .trim();
            let id = if suffix.is_empty() {
                base.to_owned()
            } else {
                format!("{base}/{suffix}")
            };

            let nifs = paths
                .iter()
                .filter(|p| {
                    let Some((parent, name)) = p.rsplit_once('/') else {
                        return false;
                    };
                    (parent == folder || parent == dir)
                        && name.ends_with(".nif")
                        && !EXCLUDED_NIF_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
                })
                .filter(|p| {
                    // Weight variants: keep `_0` only.
                    p.strip_suffix("_1.nif")
                        .is_none_or(|stem| !paths.contains(&format!("{stem}_0.nif")))
                })
                .cloned()
                .collect();

            Some(Actor {
                id,
                dir: dir.to_owned(),
                skeleton: path.clone(),
                nifs,
            })
        })
        .collect();

    // Several skeletons in one folder (draugr: skeleton, skeletonf, skeletons).
    let ids: Vec<String> = actors.iter().map(|a| a.id.clone()).collect();
    for actor in &mut actors {
        let stem = actor.skeleton.rsplit('/').next().unwrap_or_default().trim_end_matches(".hkx");
        if stem != "skeleton" && ids.iter().filter(|id| **id == actor.id).count() > 1 {
            actor.id = format!("{}/{stem}", actor.id);
        }
        if let Some((_, names)) = DEFAULT_MESHES.iter().find(|(id, _)| *id == actor.id) {
            let assets = format!("{}/character assets", actor.dir);
            actor.nifs = paths
                .iter()
                .filter(|p| {
                    p.starts_with(&assets)
                        && p.rsplit_once('/').is_some_and(|(_, name)| names.contains(&name))
                })
                .cloned()
                .collect();
        }
    }

    actors.sort_by(|a, b| a.id.cmp(&b.id));
    actors
}

/// NIFs never used as actor meshes.
const EXCLUDED_NIF_PREFIXES: &[&str] = &["skeleton", "1stperson", "decapitated"];

/// Actors whose folder holds many race/gender variants: the meshes used by default.
const DEFAULT_MESHES: &[(&str, &[&str])] = &[
    (
        "character",
        &["malebody_0.nif", "malehands_0.nif", "malefeet_0.nif", "malehead.nif"],
    ),
    (
        "character/female",
        &["femalebody_0.nif", "femalehands_0.nif", "femalefeet_0.nif", "femalehead.nif"],
    ),
];

/// Keeps the NIFs whose file name contains one of `filters` (all when empty).
pub fn filter_nifs(nifs: &[String], filters: &[String]) -> Vec<String> {
    if filters.is_empty() {
        return nifs.to_vec();
    }
    nifs.iter()
        .filter(|p| {
            let name = p.rsplit('/').next().unwrap_or(p);
            filters.iter().any(|f| name.contains(&f.to_lowercase()))
        })
        .cloned()
        .collect()
}

/// Finds an actor by id (`canine/wolf`) or directory (`cow`, `meshes/actors/cow`).
///
/// # Errors
/// If no actor or several actors match.
pub fn find_actor<'a>(actors: &'a [Actor], query: &str) -> Result<&'a Actor, Error> {
    let query = query.replace('\\', "/").trim_matches('/').to_lowercase();
    let query = query.strip_prefix("meshes/actors/").unwrap_or(&query);

    if let Some(actor) = actors.iter().find(|a| a.id == query) {
        return Ok(actor);
    }
    let by_dir: Vec<_> = actors
        .iter()
        .filter(|a| a.dir.strip_prefix("meshes/actors/") == Some(query))
        .collect();
    match by_dir.as_slice() {
        [actor] => Ok(actor),
        [] => {
            let similar: Vec<_> = actors
                .iter()
                .map(|a| a.id.as_str())
                .filter(|id| id.contains(query) || query.contains(id.rsplit('/').next().unwrap_or(id)))
                .collect();
            let hint = if similar.is_empty() {
                String::new()
            } else {
                format!("\nDid you mean: {}?", similar.join(", "))
            };
            Err(invalid_input(format!(
                "actor '{query}' not found.{hint}\nAvailable actors:\n  {}\n(`hkxc exportall --list` shows details)",
                actors.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join("\n  ")
            )))
        }
        several => Err(invalid_input(format!(
            "actor '{query}' has several skeletons; use one of: {}",
            several.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// `.../meshes/actors/<actor>/animations/...` -> `(meshes/actors/<actor>, on-disk actor dir)`.
pub fn actor_dir_from_path(path: &Path) -> Option<(String, PathBuf)> {
    let components: Vec<_> = path.components().collect();
    let lower: Vec<String> = components
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();

    let meshes = lower.windows(2).position(|w| w[0] == "meshes" && w[1] == "actors")?;
    let animations = lower.iter().skip(meshes + 2).position(|c| c == "animations")? + meshes + 2;
    if animations <= meshes + 2 {
        return None;
    }

    let dir = format!("meshes/actors/{}", lower[meshes + 2..animations].join("/"));
    let disk: PathBuf = components[..animations].iter().collect();
    Some((dir, disk))
}

/// Skeletons in a mod tree: `<actor dir>/character assets*/skeleton*.hkx`.
fn loose_skeletons(actor_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(actor_dir) else {
        return Vec::new();
    };
    let mut skeletons: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            e.path().is_dir() && (name.starts_with("character assets") || name == "characterassets")
        })
        .filter_map(|e| std::fs::read_dir(e.path()).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            name.starts_with("skeleton") && (name.ends_with(".hkx") || name.ends_with(".xml"))
        })
        .collect();
    skeletons.sort();
    skeletons
}

/// Finds Skyrim (SE, then LE) `Data` through the registry and Steam libraries.
pub fn find_skyrim_data() -> Option<PathBuf> {
    fn reg_query(key: &str, value: &str) -> Option<String> {
        let output = std::process::Command::new("reg")
            .args(["query", key, "/v", value])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().find(|l| l.trim_start().starts_with(value))?;
        Some(line.split("REG_SZ").nth(1)?.trim().to_owned())
    }

    let mut candidates = Vec::new();
    for key in [
        r"HKLM\SOFTWARE\WOW6432Node\Bethesda Softworks\Skyrim Special Edition",
        r"HKLM\SOFTWARE\WOW6432Node\Bethesda Softworks\Skyrim",
    ] {
        if let Some(path) = reg_query(key, "installed path") {
            candidates.push(PathBuf::from(path).join("Data"));
        }
    }

    let steam = reg_query(r"HKCU\Software\Valve\Steam", "SteamPath")
        .map_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from);
    let mut libraries = vec![steam.clone()];
    if let Ok(vdf) = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for line in vdf.lines() {
            let mut parts = line.split('"').filter(|s| !s.trim().is_empty());
            if parts.next() == Some("path")
                && let Some(path) = parts.next()
            {
                libraries.push(PathBuf::from(path.replace(r"\\", r"\")));
            }
        }
    }
    for library in libraries {
        for game in ["Skyrim Special Edition", "Skyrim"] {
            candidates.push(library.join("steamapps").join("common").join(game).join("Data"));
        }
    }

    candidates.into_iter().find(|path| path.is_dir())
}

/// A skeleton plus the meshes to add to exported FBX files.
#[derive(Debug)]
pub struct Rig {
    pub id: String,
    pub skeleton: Arc<Skeleton>,
    /// NIF paths inside the Data directory.
    pub nifs: Vec<String>,
}

/// Resolves skeletons per animation, caching every rig and the Data directory.
pub struct RigResolver {
    args: RigArgs,
    explicit: Option<Arc<Rig>>,
    data: OnceLock<Result<Arc<DataDir>, String>>,
    actors: OnceLock<Vec<Actor>>,
    cache: HashMap<String, Arc<Rig>>,
    /// `<OUTPUT>/data`: files read from the Data directory are copied here
    /// (same layout as `Data`) and read from here first on later runs.
    extract_dir: Option<PathBuf>,
}

/// Name of the extracted-data directory inside an output directory.
pub const EXTRACT_DIR_NAME: &str = "data";

impl RigResolver {
    /// # Errors
    /// If the explicit skeleton cannot be read or decoded.
    pub fn new(args: &RigArgs, skeleton: Option<&Path>) -> Result<Self, Error> {
        let explicit = skeleton
            .map(|path| -> Result<_, Error> {
                Ok(Arc::new(Rig {
                    id: path.display().to_string(),
                    skeleton: Arc::new(read_skeleton(path)?),
                    nifs: Vec::new(),
                }))
            })
            .transpose()?;

        if explicit.is_none() && args.actor.is_none() && !args.auto {
            return Err(invalid_input("one of --skeleton, --actor or --auto is required"));
        }

        Ok(Self {
            args: args.clone(),
            explicit,
            data: OnceLock::new(),
            actors: OnceLock::new(),
            cache: HashMap::new(),
            extract_dir: None,
        })
    }

    /// Copies every file read from the Data directory to `dir` and reads it
    /// from there first (see [`EXTRACT_DIR_NAME`]).
    #[must_use]
    pub fn with_extract_dir(mut self, dir: PathBuf) -> Self {
        self.extract_dir = Some(dir);
        self
    }

    pub fn extract_dir(&self) -> Option<&Path> {
        self.extract_dir.as_deref()
    }

    /// The Data directory (opened on first use), with the extract directory on top.
    ///
    /// Without a Skyrim installation, an existing extract directory alone is used.
    ///
    /// # Errors
    /// If no Data directory is given or found, or it cannot be opened.
    pub fn data(&self) -> Result<Arc<DataDir>, Error> {
        self.data
            .get_or_init(|| {
                let extract = self.extract_dir.clone().filter(|dir| dir.is_dir());
                let data = match self.args.data.clone().or_else(find_skyrim_data) {
                    Some(path) => {
                        tracing::info!("Data directory: {}", path.display());
                        let data = DataDir::open(&path).map_err(|e| e.to_string())?;
                        match &self.extract_dir {
                            Some(dir) => data.with_overlay(dir),
                            None => data,
                        }
                    }
                    None => {
                        let dir = extract.ok_or("Skyrim Data directory not found; pass --data <DIR>")?;
                        tracing::info!("Data directory not found; using extracted data: {}", dir.display());
                        DataDir::open(&dir).map_err(|e| e.to_string())?
                    }
                };
                Ok(Arc::new(data))
            })
            .clone()
            .map_err(invalid_input)
    }

    /// Reads `path` from the Data directory and copies it to the extract directory.
    ///
    /// # Errors
    /// If the file is missing or cannot be read or written.
    pub fn read_data(&self, path: &str) -> Result<Vec<u8>, Error> {
        let bytes = self
            .data()?
            .read(path)
            .map_err(|e| invalid_input(e.to_string()))?
            .ok_or_else(|| invalid_input(format!("missing {path}")))?;
        self.extract(path, &bytes)?;
        Ok(bytes)
    }

    /// Writes `bytes` to `<extract dir>/<path>` unless it already exists.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn extract(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        if let Some(dir) = &self.extract_dir {
            let target = dir.join(serde_bsa::normalize(path));
            if !target.exists() {
                super::write_file_atomic(&target, bytes)?;
            }
        }
        Ok(())
    }

    /// # Errors
    /// If the Data directory cannot be opened.
    pub fn actors(&self) -> Result<&[Actor], Error> {
        if let Some(actors) = self.actors.get() {
            return Ok(actors);
        }
        let actors = discover_actors(&*self.data()?);
        Ok(self.actors.get_or_init(|| actors))
    }

    /// Skeleton for the animation at `path`.
    ///
    /// # Errors
    /// If the actor cannot be determined or its skeleton cannot be decoded.
    pub fn resolve(&mut self, path: &Path) -> Result<Arc<Rig>, Error> {
        if let Some(rig) = &self.explicit {
            return Ok(Arc::clone(rig));
        }

        if let Some(query) = self.args.actor.clone() {
            let actor = find_actor(self.actors()?, &query)?.clone();
            return self.actor_rig(&actor);
        }

        // --auto
        let (dir, disk) = actor_dir_from_path(path).ok_or_else(|| {
            invalid_input(format!(
                "--auto: '{}' is not under .../meshes/actors/<actor>/animations/",
                path.display()
            ))
        })?;

        if let [skeleton] = loose_skeletons(&disk).as_slice() {
            let key = skeleton.display().to_string();
            if let Some(rig) = self.cache.get(&key) {
                return Ok(Arc::clone(rig));
            }
            let nifs = self
                .actors()
                .ok()
                .and_then(|actors| actors.iter().find(|a| a.dir == dir).map(|a| a.nifs.clone()))
                .unwrap_or_default();
            let rig = Arc::new(Rig {
                id: key.clone(),
                skeleton: Arc::new(read_skeleton(skeleton)?),
                nifs,
            });
            self.cache.insert(key, Arc::clone(&rig));
            return Ok(rig);
        }

        let query = dir.trim_start_matches("meshes/actors/").to_owned();
        let actor = find_actor(self.actors()?, &query)?.clone();
        self.actor_rig(&actor)
    }

    fn actor_rig(&mut self, actor: &Actor) -> Result<Arc<Rig>, Error> {
        if let Some(rig) = self.cache.get(&actor.id) {
            return Ok(Arc::clone(rig));
        }
        let bytes = self.read_data(&actor.skeleton)?;
        let skeleton = Skeleton::from_bytes(&bytes, Path::new(&actor.skeleton))
            .map_err(|e| invalid_input(format!("{}: {e}", actor.skeleton)))?;
        tracing::info!("Actor '{}': {} ({} meshes)", actor.id, actor.skeleton, actor.nifs.len());

        let rig = Arc::new(Rig {
            id: actor.id.clone(),
            skeleton: Arc::new(skeleton),
            nifs: actor.nifs.clone(),
        });
        self.cache.insert(actor.id.clone(), Arc::clone(&rig));
        Ok(rig)
    }
}

fn read_skeleton(path: &Path) -> Result<Skeleton, Error> {
    let bytes = std::fs::read(path).map_err(|source| Error::FailedReadFile {
        source,
        path: path.to_owned(),
    })?;
    Skeleton::from_bytes(&bytes, path).map_err(|e| invalid_input(format!("{}: {e}", path.display())))
}

/// Loads NIF meshes for FBX, embedding textures found in the Data directory.
///
/// `nif` is either a file on disk or a path inside the Data directory.
#[cfg(feature = "fbx")]
pub fn load_meshes(nif: NifSource<'_>, data: Option<&DataDir>, textures: bool) -> Result<Vec<serde_fbx::ser::Mesh>, Error> {
    let (bytes, name) = match nif {
        NifSource::File(path) => (
            std::fs::read(path).map_err(|source| Error::FailedReadFile {
                source,
                path: path.to_owned(),
            })?,
            path.display().to_string(),
        ),
        NifSource::Data(path) => (
            data.and_then(|d| d.read(path).ok().flatten())
                .ok_or_else(|| invalid_input(format!("missing {path}")))?,
            path.to_owned(),
        ),
    };
    let scene =
        serde_nif::from_bytes(&bytes).map_err(|e| invalid_input(format!("failed to read '{name}': {e}")))?;

    // Loose textures next to an on-disk NIF (`<Data>/meshes/...` -> `<Data>/textures/...`).
    let loose_root = match nif {
        NifSource::File(path) => data_root_of(path),
        NifSource::Data(_) => None,
    };

    Ok(serde_fbx::nif::meshes_from_nif(&scene, |texture| {
        if !textures {
            return None;
        }
        let found = loose_root
            .as_ref()
            .and_then(|root| std::fs::read(root.join(texture)).ok())
            .or_else(|| data.and_then(|d| d.read(texture).ok().flatten()));
        if found.is_none() {
            tracing::warn!("texture not found: {texture}");
        }
        found
    }))
}

/// How FBX meshes reference their textures.
#[cfg(feature = "fbx")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TextureMode {
    /// Embed the DDS files in every FBX (self-contained, larger files).
    Embed,
    /// Write each DDS once to `<OUTPUT>/textures/...` and reference it by a relative path.
    External,
    /// No textures.
    None,
}

/// Writes embedded texture bytes to `<root>/<texture path>` (once) and drops
/// them from the meshes, which then reference the files.
///
/// # Errors
/// If a texture file cannot be written.
#[cfg(feature = "fbx")]
pub fn externalize_textures(meshes: &mut [serde_fbx::ser::Mesh], root: &Path) -> Result<(), Error> {
    for texture in meshes.iter_mut().flat_map(|m| m.textures.iter_mut()) {
        if let Some(content) = texture.content.take() {
            let path = root.join(&texture.path);
            if !path.exists() {
                super::write_file_atomic(&path, &content)?;
            }
        }
    }
    Ok(())
}

/// Meshes whose texture paths are relative to the directory of `fbx`,
/// for textures written under `root` by [`externalize_textures`].
#[cfg(feature = "fbx")]
pub fn relative_textures(meshes: &[serde_fbx::ser::Mesh], fbx: &Path, root: &Path) -> Vec<serde_fbx::ser::Mesh> {
    let dir = fbx.parent().unwrap_or_else(|| Path::new(""));
    let prefix = relative_dir(dir, root);
    let mut meshes = meshes.to_vec();
    for texture in meshes.iter_mut().flat_map(|m| m.textures.iter_mut()) {
        texture.path = format!("{prefix}{}", texture.path);
    }
    meshes
}

/// Relative directory prefix leading from `from` to `to`, with a trailing `/`,
/// e.g. `out/cow/sub` -> `out/data` = `../../data/`.
#[cfg(feature = "fbx")]
fn relative_dir(from: &Path, to: &Path) -> String {
    let absolute = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (from, to) = (absolute(from), absolute(to));
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();

    let mut prefix = "../".repeat(from.len() - common);
    for component in &to[common..] {
        prefix.push_str(&component.as_os_str().to_string_lossy());
        prefix.push('/');
    }
    prefix
}

#[cfg(feature = "fbx")]
#[derive(Debug, Clone, Copy)]
pub enum NifSource<'a> {
    File(&'a Path),
    Data(&'a str),
}

/// `.../Data/meshes/actors/cow/x.nif` -> `.../Data`.
#[cfg(feature = "fbx")]
fn data_root_of(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("meshes")))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "fbx")]
    #[test]
    fn relative_dir_between_siblings() {
        assert_eq!(relative_dir(Path::new("out/cow/sub"), Path::new("out/data")), "../../data/");
        assert_eq!(relative_dir(Path::new("out"), Path::new("out")), "");
    }

    #[test]
    fn actor_dir_from_mod_path() {
        let (dir, disk) =
            actor_dir_from_path(Path::new("mods/x/Meshes/Actors/dlc01/ChaurusFlyer/Animations/sub/a.hkx")).unwrap();
        assert_eq!(dir, "meshes/actors/dlc01/chaurusflyer");
        assert_eq!(disk, PathBuf::from("mods/x/Meshes/Actors/dlc01/ChaurusFlyer"));
        assert!(actor_dir_from_path(Path::new("anims/a.hkx")).is_none());
    }
}
