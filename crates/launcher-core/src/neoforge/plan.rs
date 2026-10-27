//! Pure, bounded client processor planning from authenticated installer/game inputs.
use super::*;
use crate::minecraft::{Artifact, JavaRequirement, VersionMetadata};

#[derive(Clone, Debug)]
pub enum InputSource {
    Library(LibraryArtifact),
    Mojang(Artifact),
    Embedded {
        entry: String,
        sha1: String,
        size: u64,
    },
}

#[derive(Clone, Debug)]
pub struct PlannedInput {
    pub destination: String,
    pub source: InputSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessorArgument {
    Literal(String),
    /// Portable path relative to the private operation directory.
    Path(String),
}

#[derive(Clone, Debug)]
pub struct PlannedProcessor {
    pub(crate) jar: String,
    pub(crate) classpath: Vec<String>,
    pub(crate) args: Vec<ProcessorArgument>,
    pub(crate) outputs: BTreeMap<String, Option<String>>,
}

impl PlannedProcessor {
    pub fn jar(&self) -> &str {
        &self.jar
    }
    pub fn classpath(&self) -> &[String] {
        &self.classpath
    }
    pub fn arguments(&self) -> &[ProcessorArgument] {
        &self.args
    }
    pub fn outputs(&self) -> impl Iterator<Item = &str> {
        self.outputs.keys().map(String::as_str)
    }
}

/// Fields are private: execution accepts only plans resolved from verified bytes.
#[derive(Clone, Debug)]
pub struct ProcessorPlan {
    pub(crate) identity: InstallerIdentity,
    pub(crate) java: JavaRequirement,
    pub(crate) inputs: Vec<PlannedInput>,
    pub(crate) processors: Vec<PlannedProcessor>,
    pub(crate) launch: LaunchProfile,
    pub(crate) derived: BTreeSet<String>,
}

impl ProcessorPlan {
    pub fn derived_paths(&self) -> impl Iterator<Item = &str> {
        self.derived.iter().map(String::as_str)
    }
    pub fn inputs(&self) -> &[PlannedInput] {
        &self.inputs
    }
    pub fn processors(&self) -> &[PlannedProcessor] {
        &self.processors
    }
    pub fn java(&self) -> &JavaRequirement {
        &self.java
    }
    pub fn launch_profile(&self) -> &LaunchProfile {
        &self.launch
    }
    pub fn identity(&self) -> &InstallerIdentity {
        &self.identity
    }

    pub fn resolve(
        installer: &[u8],
        identity: &InstallerIdentity,
        game: &[u8],
        game_source: &Artifact,
    ) -> Result<Self, CoreError> {
        let metadata = InstallerMetadata::decode(installer, identity)?;
        let game = VersionMetadata::decode(game, game_source, &identity.minecraft)?;
        if game.inherits_from.is_some() {
            return Err(unsupported());
        }
        let java = game.java_version.ok_or_else(unsupported)?;
        let client = game.downloads.get("client").ok_or_else(invalid)?.clone();
        let mappings = game
            .downloads
            .get("client_mappings")
            .ok_or_else(unsupported)?
            .clone();
        client.validate()?;
        mappings.validate()?;
        let mut inputs = BTreeMap::new();
        for library in metadata
            .install
            .libraries
            .iter()
            .chain(&metadata.launch.libraries)
        {
            let artifact = library.downloads.artifact.clone();
            insert_input(
                &mut inputs,
                format!("libraries/{}", artifact.path),
                InputSource::Library(artifact),
            )?;
        }
        insert_input(
            &mut inputs,
            "minecraft-client.jar".into(),
            InputSource::Mojang(client),
        )?;
        let mut variables = BTreeMap::new();
        for (key, value) in &metadata.install.data {
            let value = &value.client;
            let arg = if let Some(coordinate) =
                value.strip_prefix('[').and_then(|v| v.strip_suffix(']'))
            {
                ProcessorArgument::Path(format!(
                    "libraries/{}",
                    Coordinate::parse(coordinate)?.path()
                ))
            } else if let Some(literal) =
                value.strip_prefix('\'').and_then(|v| v.strip_suffix('\''))
            {
                ProcessorArgument::Literal(literal.into())
            } else {
                let entry = value.strip_prefix('/').ok_or_else(invalid)?;
                let mut archive =
                    zip::ZipArchive::new(Cursor::new(installer)).map_err(|_| invalid())?;
                let bytes = super::read_entry(&mut archive, entry)?;
                let destination = format!("embedded/{entry}");
                insert_input(
                    &mut inputs,
                    destination.clone(),
                    InputSource::Embedded {
                        entry: entry.into(),
                        sha1: format!("{:x}", Sha1::digest(&bytes)),
                        size: bytes.len() as u64,
                    },
                )?;
                ProcessorArgument::Path(destination)
            };
            variables.insert(key.clone(), arg);
        }
        variables.insert("SIDE".into(), ProcessorArgument::Literal("client".into()));
        variables.insert(
            "MINECRAFT_VERSION".into(),
            ProcessorArgument::Literal(identity.minecraft.clone()),
        );
        variables.insert(
            "MINECRAFT_JAR".into(),
            ProcessorArgument::Path("minecraft-client.jar".into()),
        );
        let mappings_path = variable_path(&variables, "MOJMAPS")?;
        insert_input(&mut inputs, mappings_path, InputSource::Mojang(mappings))?;

        let mut processors = Vec::new();
        let mut produced = BTreeSet::new();
        let mut mapping_steps = 0;
        let mut available: BTreeSet<String> = inputs.keys().cloned().collect();
        let mut shapes: BTreeSet<String> = inputs
            .keys()
            .map(|path| path.to_ascii_lowercase())
            .collect();
        for processor in metadata
            .install
            .processors
            .iter()
            .filter(|processor| processor.runs_on_client())
        {
            let shape = task_shape(processor)?;
            let args = processor
                .args
                .iter()
                .map(|arg| resolve_argument(arg, &variables))
                .collect::<Result<Vec<_>, _>>()?;
            if shape.mapping_download {
                // This exact network step is satisfied by the authenticated Mojang input.
                if args.get(3) != Some(&ProcessorArgument::Literal(identity.minecraft.clone())) {
                    return Err(invalid());
                }
                mapping_steps += 1;
                continue;
            }
            let mut outputs = BTreeMap::new();
            for index in shape.outputs {
                let path = argument_path(&args[*index])?.to_owned();
                if !shapes.insert(path.to_ascii_lowercase()) {
                    return Err(invalid());
                }
                produced.insert(path.clone());
                outputs.insert(path, None);
            }
            for (index, arg) in args.iter().enumerate() {
                if let ProcessorArgument::Path(path) = arg
                    && !shape.outputs.contains(&index)
                    && !available.contains(path)
                {
                    return Err(invalid());
                }
            }
            for (output, hash) in &processor.outputs {
                let path = argument_path(&resolve_argument(output, &variables)?)?.to_owned();
                let hash = match resolve_argument(hash, &variables)? {
                    ProcessorArgument::Literal(hash) if valid_hash(&hash) => hash,
                    _ => return Err(invalid()),
                };
                let slot = outputs.get_mut(&path).ok_or_else(invalid)?;
                *slot = Some(hash);
            }
            available.extend(outputs.keys().cloned());
            let jar = format!("libraries/{}", Coordinate::parse(&processor.jar)?.path());
            let mut classpath = vec![jar.clone()];
            for coordinate in &processor.classpath {
                let path = format!("libraries/{}", Coordinate::parse(coordinate)?.path());
                if !classpath.contains(&path) {
                    classpath.push(path);
                }
            }
            if classpath.iter().any(|path| !inputs.contains_key(path)) {
                return Err(invalid());
            }
            processors.push(PlannedProcessor {
                jar,
                classpath,
                args,
                outputs,
            });
        }
        if mapping_steps != 1
            || processors.is_empty()
            || !produced.contains(&variable_path(&variables, "PATCHED")?)
            || !produced.contains(&variable_path(&variables, "MC_EXTRA")?)
        {
            return Err(invalid());
        }
        validate_destinations(shapes.iter().map(String::as_str))?;
        Ok(Self {
            identity: identity.clone(),
            java,
            inputs: inputs.into_values().collect(),
            processors,
            launch: metadata.launch,
            derived: produced,
        })
    }
}

fn unsupported() -> CoreError {
    CoreError::new(ErrorCode::UnsupportedInstance)
}

fn argument_path(argument: &ProcessorArgument) -> Result<&str, CoreError> {
    if let ProcessorArgument::Path(path) = argument {
        safe_path(path)?;
        Ok(path)
    } else {
        Err(invalid())
    }
}
fn variable_path(
    variables: &BTreeMap<String, ProcessorArgument>,
    key: &str,
) -> Result<String, CoreError> {
    Ok(argument_path(variables.get(key).ok_or_else(invalid)?)?.into())
}
fn resolve_argument(
    value: &str,
    variables: &BTreeMap<String, ProcessorArgument>,
) -> Result<ProcessorArgument, CoreError> {
    if let Some(key) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
        return variables.get(key).cloned().ok_or_else(unsupported);
    }
    if let Some(coordinate) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        return Ok(ProcessorArgument::Path(format!(
            "libraries/{}",
            Coordinate::parse(coordinate)?.path()
        )));
    }
    if value.contains(['{', '}', '[', ']']) {
        return Err(unsupported());
    }
    Ok(ProcessorArgument::Literal(value.into()))
}

fn insert_input(
    inputs: &mut BTreeMap<String, PlannedInput>,
    destination: String,
    source: InputSource,
) -> Result<(), CoreError> {
    safe_path(&destination)?;
    if let Some(old) = inputs.get(&destination) {
        // The same library can be required by both installer and launch profiles.
        return match (&old.source, &source) {
            (InputSource::Library(a), InputSource::Library(b))
                if a.sha1 == b.sha1 && a.size == b.size =>
            {
                Ok(())
            }
            _ => Err(invalid()),
        };
    }
    inputs.insert(
        destination.clone(),
        PlannedInput {
            destination,
            source,
        },
    );
    Ok(())
}

fn validate_destinations<'a>(paths: impl Iterator<Item = &'a str>) -> Result<(), CoreError> {
    let paths: BTreeSet<_> = paths.collect();
    for path in &paths {
        for (end, _) in path.match_indices('/') {
            if paths.contains(&path[..end]) {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

struct TaskShape {
    outputs: &'static [usize],
    mapping_download: bool,
}

// Interpret only the independently implemented client task grammar. Unknown
// processor behaviors fail closed instead of receiving arbitrary path arguments.
fn task_shape(processor: &Processor) -> Result<TaskShape, CoreError> {
    let args: Vec<_> = processor.args.iter().map(String::as_str).collect();
    let coordinate: Vec<_> = processor.jar.split(':').collect();
    let family = coordinate.get(..2).ok_or_else(invalid)?;
    let (shape, outputs, mapping_download): (&[&str], &[usize], bool) = match family {
        ["net.neoforged.installertools", "installertools"] => match args.get(1) {
            Some(&"MCP_DATA") => (
                &[
                    "--task",
                    "MCP_DATA",
                    "--input",
                    "*coordinate*",
                    "--output",
                    "{MAPPINGS}",
                    "--key",
                    "mappings",
                ],
                &[5],
                false,
            ),
            Some(&"DOWNLOAD_MOJMAPS") => (
                &[
                    "--task",
                    "DOWNLOAD_MOJMAPS",
                    "--version",
                    "*version*",
                    "--side",
                    "{SIDE}",
                    "--output",
                    "{MOJMAPS}",
                ],
                &[],
                true,
            ),
            Some(&"MERGE_MAPPING") => (
                &[
                    "--task",
                    "MERGE_MAPPING",
                    "--left",
                    "{MAPPINGS}",
                    "--right",
                    "{MOJMAPS}",
                    "--output",
                    "{MERGED_MAPPINGS}",
                    "--classes",
                    "--fields",
                    "--methods",
                    "--reverse-right",
                ],
                &[7],
                false,
            ),
            _ => return Err(unsupported()),
        },
        ["net.neoforged.installertools", "jarsplitter"] => (
            &[
                "--input",
                "{MINECRAFT_JAR}",
                "--slim",
                "{MC_SLIM}",
                "--extra",
                "{MC_EXTRA}",
                "--srg",
                "{MERGED_MAPPINGS}",
            ],
            &[3, 5],
            false,
        ),
        ["net.neoforged", "AutoRenamingTool"] => (
            &[
                "--input",
                "{MC_SLIM}",
                "--output",
                "{MC_SRG}",
                "--names",
                "{MERGED_MAPPINGS}",
                "--ann-fix",
                "--ids-fix",
                "--src-fix",
                "--record-fix",
            ],
            &[3],
            false,
        ),
        ["net.neoforged.installertools", "binarypatcher"] => (
            &[
                "--clean",
                "{MC_SRG}",
                "--output",
                "{PATCHED}",
                "--apply",
                "{BINPATCH}",
            ],
            &[3],
            false,
        ),
        _ => return Err(unsupported()),
    };
    if args.len() != shape.len()
        || args
            .iter()
            .zip(shape)
            .any(|(actual, expected)| match *expected {
                "*coordinate*" => !actual.starts_with('[') || !actual.ends_with(']'),
                "*version*" => !component(actual),
                expected => *actual != expected,
            })
    {
        return Err(unsupported());
    }
    Ok(TaskShape {
        outputs,
        mapping_download,
    })
}

#[cfg(test)]
mod tests;
