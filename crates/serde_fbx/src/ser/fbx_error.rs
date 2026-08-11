#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("ufbxw element not found in {function}: {description}"))]
    ElementNotFound {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw element has wrong type in {function}: {description}"))]
    ElementWrongType {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw element type not found in {function}: {description}"))]
    ElementTypeNotFound {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw property has wrong data type in {function}: {description}"))]
    PropDataType {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw property not found in {function}: {description}"))]
    PropNotFound {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw wrong data type in {function}: {description}"))]
    WrongDataType {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw index out of bounds in {function}: {description}"))]
    IndexOutOfBounds {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw cyclical parent in {function}: {description}"))]
    CyclicalParent {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw buffer not found in {function}: {description}"))]
    BufferNotFound {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw buffer has wrong type in {function}: {description}"))]
    BufferWrongType {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw buffer is not editable in {function}: {description}"))]
    BufferNotEditable {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw fatal error in {function}: {description}"))]
    Fatal {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw string is too long in {function}: {description}"))]
    StringTooLong {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw memory limit exceeded in {function}: {description}"))]
    MemoryLimit {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw allocation limit exceeded in {function}: {description}"))]
    AllocationLimit {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw allocation failed in {function}: {description}"))]
    AllocationFailure {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw file size limit exceeded in {function}: {description}"))]
    FileSizeLimit {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw buffer stream error in {function}: {description}"))]
    BufferStream {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw write failed in {function}: {description}"))]
    WriteFailed {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw deflate failed in {function}: {description}"))]
    DeflateFailed {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw array is too big in {function}: {description}"))]
    ArrayTooBig {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw path is too long in {function}: {description}"))]
    PathTooLong {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw file open failed in {function}: {description}"))]
    FileOpenFailed {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw ASCII format error in {function}: {description}"))]
    AsciiFormat {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw thread sync initialization failed in {function}: {description}"))]
    ThreadSyncInit {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw thread pool initialization failed in {function}: {description}"))]
    ThreadPoolInit {
        function: String,
        description: String,
    },

    #[snafu(display("ufbxw stream begin failed in {function}: {description}"))]
    StreamBegin {
        function: String,
        description: String,
    },

    #[snafu(display("unknown ufbxw error type {kind} in {function}: {description}"))]
    Unknown {
        kind: ufbx_write::sys::ufbxw_error_type,
        function: String,
        description: String,
    },
}

fn ufbxw_string_to_string(value: &ufbx_write::sys::ufbxw_string) -> String {
    if value.data.is_null() || value.length == 0 {
        return String::new();
    }
    let bytes = unsafe { core::slice::from_raw_parts(value.data.cast::<u8>(), value.length) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn ufbxw_description_to_string(error: &ufbx_write::sys::ufbxw_error) -> String {
    let length = error.description_length.min(error.description.len());
    if length == 0 {
        return String::new();
    }
    let bytes =
        unsafe { core::slice::from_raw_parts(error.description.as_ptr().cast::<u8>(), length) };
    String::from_utf8_lossy(bytes).into_owned()
}

impl From<ufbx_write::sys::ufbxw_error> for Error {
    fn from(error: ufbx_write::sys::ufbxw_error) -> Self {
        use ufbx_write::sys;

        let function = ufbxw_string_to_string(&error.function);
        let description = ufbxw_description_to_string(&error);

        match error.type_ {
            sys::ufbxw_error_type_UFBXW_ERROR_ELEMENT_NOT_FOUND => Self::ElementNotFound {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_ELEMENT_WRONG_TYPE => Self::ElementWrongType {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_ELEMENT_TYPE_NOT_FOUND => Self::ElementTypeNotFound {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_PROP_DATA_TYPE => Self::PropDataType {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_PROP_NOT_FOUND => Self::PropNotFound {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_WRONG_DATA_TYPE => Self::WrongDataType {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_INDEX_OUT_OF_BOUNDS => Self::IndexOutOfBounds {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_CYCLICAL_PARENT => Self::CyclicalParent {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_BUFFER_NOT_FOUND => Self::BufferNotFound {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_BUFFER_WRONG_TYPE => Self::BufferWrongType {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_BUFFER_NOT_EDITABLE => Self::BufferNotEditable {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_FATAL => Self::Fatal {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_STRING_TOO_LONG => Self::StringTooLong {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_MEMORY_LIMIT => Self::MemoryLimit {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_ALLOCATION_LIMIT => Self::AllocationLimit {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_ALLOCATION_FAILURE => Self::AllocationFailure {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_FILE_SIZE_LIMIT => Self::FileSizeLimit {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_BUFFER_STREAM => Self::BufferStream {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_WRITE_FAILED => Self::WriteFailed {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_DEFLATE_FAILED => Self::DeflateFailed {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_ARRAY_TOO_BIG => Self::ArrayTooBig {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_PATH_TOO_LONG => Self::PathTooLong {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_FILE_OPEN_FAILED => Self::FileOpenFailed {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_ASCII_FORMAT => Self::AsciiFormat {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_THREAD_SYNC_INIT => Self::ThreadSyncInit {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_THREAD_POOL_INIT => Self::ThreadPoolInit {
                function,
                description,
            },
            sys::ufbxw_error_type_UFBXW_ERROR_STREAM_BEGIN => Self::StreamBegin {
                function,
                description,
            },
            kind => Self::Unknown {
                kind,
                function,
                description,
            },
        }
    }
}
