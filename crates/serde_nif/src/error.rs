use snafu::Snafu;

/// Errors produced while reading a NIF file.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// The file does not start with a Gamebryo/NetImmerse header line.
    #[snafu(display("not a NIF file: missing header line"))]
    InvalidHeader,

    /// Only Skyrim LE/SE files (20.2.0.7, user version 12) are supported.
    #[snafu(display(
        "unsupported NIF version {version:#010x} (user {user_version}, bs {bs_version}); \
         only Skyrim LE/SE (20.2.0.7, user 12) is supported"
    ))]
    UnsupportedVersion {
        version: u32,
        user_version: u32,
        bs_version: u32,
    },

    /// The data ended before a value could be read.
    #[snafu(display("unexpected end of data at {offset:#x} (needed {len} bytes)"))]
    UnexpectedEof { offset: usize, len: usize },

    /// A block could not be parsed.
    #[snafu(display("failed to read block {index} ({block_type}): {source}"))]
    Block {
        index: usize,
        block_type: String,
        source: Box<Self>,
    },

    /// The data of a block is inconsistent.
    #[snafu(display("{message}"))]
    InvalidData { message: String },
}
