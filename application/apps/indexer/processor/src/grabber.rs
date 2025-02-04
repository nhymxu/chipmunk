use indexer_base::progress::ComputationResult;
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs,
    io::{Read, Write},
    ops::RangeInclusive,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub trait GrabTrait {
    fn grab_content(&self, line_range: &LineRange) -> Result<GrabbedContent, GrabError>;
    fn inject_metadata(&mut self, metadata: GrabMetadata) -> Result<(), GrabError>;
    fn get_metadata(&self) -> Option<&GrabMetadata>;
    fn drop_metadata(&mut self);
    fn associated_file(&self) -> PathBuf;
}

pub trait AsyncGrabTrait: GrabTrait + Sync + Send + std::fmt::Debug {}

#[derive(Error, Debug)]
pub enum GrabError {
    #[error("Configuration error ({0})")]
    Config(String),
    #[error("Channel-Communication error ({0})")]
    Communication(String),
    #[error("IO error while grabbing: ({0:?})")]
    IoOperation(#[from] std::io::Error),
    #[error("Invalid range: ({range:?}) ({context})")]
    InvalidRange { range: LineRange, context: String },
    #[error("Grabber interrupted")]
    Interrupted,
    #[error("Metadata initialization not done")]
    NotInitialize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrabbedElement {
    #[serde(rename = "id")]
    pub source_id: String,
    #[serde(rename = "c")]
    pub content: String,
    #[serde(rename = "r")]
    pub row: Option<usize>,
    #[serde(rename = "p")]
    pub pos: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrabbedContent {
    pub grabbed_elements: Vec<GrabbedElement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ByteIdentifier;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineIdentifier;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct GrabRange<T> {
    pub range: RangeInclusive<u64>,
    phantom: std::marker::PhantomData<T>,
}
pub type ByteRange = GrabRange<ByteIdentifier>;
pub type LineRange = GrabRange<LineIdentifier>;

impl<T> fmt::Debug for GrabRange<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.range)
    }
}

impl<T> GrabRange<T> {
    pub fn from(range: RangeInclusive<u64>) -> Self {
        Self {
            range,
            phantom: std::marker::PhantomData,
        }
    }

    pub fn single_line(index: u64) -> Self {
        Self {
            range: (index..=index),
            phantom: std::marker::PhantomData,
        }
    }

    pub fn start(&self) -> u64 {
        *self.range.start()
    }

    pub fn end(&self) -> u64 {
        *self.range.end()
    }

    pub fn size(&self) -> u64 {
        if self.range.is_empty() {
            return 0;
        }
        self.range.end() - self.range.start() + 1
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub bytes: ByteRange,
    pub lines: LineRange,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrabMetadata {
    /// mapping that contains the byte offset -> newline count in the section
    /// e.g.    (0,999) -> 233 (233 newlines in [0,999])
    ///         (1000,1999) -> 199 (199 newlines in [1000,1999])
    pub slots: Vec<Slot>,
    pub line_count: usize,
}

/// A trait that defines how indexed content can be retrieved from a certain file type.
///
/// When any content should be displayed in chipmunk, it needs to be visualized
/// as a sequence of lines of text that represent the content. This trait needs to be
/// implemented for each file type that we support. It defines everything that is needed to
/// calculate cached metadata that will subsequently be used for extracting parts of the
/// file in question
pub trait MetadataSource {
    fn source_id(&self) -> String;

    /// This will initialize the cached metadata from a file
    fn from_file(
        &self,
        shutdown_receiver: Option<cc::Receiver<()>>,
    ) -> Result<ComputationResult<GrabMetadata>, GrabError>;

    /// Calling this function is only possible when the metadata already has been
    /// created.
    /// It will deliever the content of the file that is requested by line_range.
    fn get_entries(
        &self,
        metadata: &GrabMetadata,
        line_range: &LineRange,
    ) -> Result<GrabbedContent, GrabError>;

    /// will return the number of log entries in a file.
    fn count_lines(&self) -> Result<usize, GrabError>;

    /// the size of the input content
    fn input_size(&self) -> Result<u64, GrabError> {
        let input_file_size = std::fs::metadata(&self.path())
            .map_err(|e| {
                GrabError::Config(format!("Could not determine size of input file: {}", e))
            })?
            .len();
        Ok(input_file_size)
    }

    /// the path of the file that is the source for the content
    fn path(&self) -> &Path;
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Slot: bytes: [{:?}], lines: [{:?}]",
            self.bytes.range, self.lines.range,
        )
    }
}

impl Slot {
    pub fn byte_count(&self) -> u64 {
        self.bytes.size()
    }
    pub fn line_count(&self) -> u64 {
        self.lines.size()
    }
}

pub trait LogItem {
    fn id(&self) -> String;
}

impl LogItem for String {
    fn id(&self) -> String {
        self.clone()
    }
}

#[derive(Debug)]
pub struct Grabber<T: MetadataSource> {
    source: T,
    pub metadata: Option<GrabMetadata>,
    pub input_file_size: u64,
}

impl<T> AsyncGrabTrait for Grabber<T> where T: MetadataSource + Sync + Send + std::fmt::Debug {}

impl<T> GrabTrait for Grabber<T>
where
    T: MetadataSource + Sync + Send + std::fmt::Debug,
{
    fn grab_content(&self, line_range: &LineRange) -> Result<GrabbedContent, GrabError> {
        self.get_entries(line_range)
    }

    fn inject_metadata(&mut self, metadata: GrabMetadata) -> Result<(), GrabError> {
        self.metadata = Some(metadata);
        Ok(())
    }

    fn get_metadata(&self) -> Option<&GrabMetadata> {
        self.metadata.as_ref()
    }

    fn drop_metadata(&mut self) {
        self.metadata = None;
    }

    fn associated_file(&self) -> PathBuf {
        self.source.path().to_path_buf()
    }
}

impl<T: MetadataSource> Grabber<T> {
    /// Create a new Grabber without creating the metadata
    /// ...
    /// A new Grabber instance can only be created if the file is non-empty,
    /// otherwise this function will return an error
    pub fn lazy(source: T) -> Result<Self, GrabError> {
        let input_file_size = source.input_size()?;
        if input_file_size == 0 {
            return Err(GrabError::Config("Cannot grab empty file".to_string()));
        }
        Ok(Self {
            source,
            metadata: None,
            input_file_size,
        })
    }

    /// If a grabber was created lazily, the metadata can be created with this
    /// function.
    pub fn create_metadata(
        &mut self,
        shutdown_rx: Option<cc::Receiver<()>>,
    ) -> Result<(), GrabError> {
        if self.metadata.is_none() {
            if let ComputationResult::Item(md) = self.source.from_file(shutdown_rx)? {
                self.metadata = Some(md)
            }
        }
        Ok(())
    }

    /// Create a new Grabber by deviding the file content into slots
    /// [0-1023] => line 0 - 23
    /// [1024-2047] => line 24 - 38
    /// ...
    /// A new Grabber instance can only be created if the file is non-empty,
    /// otherwise this function will return an error
    pub fn new(source: T) -> Result<Self, GrabError> {
        let input_file_size = source.input_size()?;
        if input_file_size == 0 {
            return Err(GrabError::Config("Cannot grab empty file".to_string()));
        }

        let metadata = match source.from_file(None)? {
            ComputationResult::Item(md) => Ok(Some(md)),
            ComputationResult::Stopped => Err(GrabError::Interrupted),
        }?;

        Ok(Self {
            source,
            metadata,
            input_file_size,
        })
    }

    /// if the metadata for a path already exists, it can be read
    /// from a file
    pub fn load_metadata(mut self, slots_path: impl AsRef<Path>) -> Result<Self, GrabError> {
        let mut slots_file = fs::File::open(&slots_path)
            .map_err(|e| GrabError::Config(format!("Could not open slot file: {}", e)))?;
        let mut buffer = vec![];
        let _bytes_read = slots_file.read_to_end(&mut buffer);
        self.metadata = Some(
            bincode::deserialize(&buffer)
                .map_err(|_| GrabError::Config("Could not deserialize metadata".to_string()))?,
        );
        Ok(self)
    }

    pub fn get_entries(&self, line_range: &LineRange) -> Result<GrabbedContent, GrabError> {
        match &self.metadata {
            None => Err(GrabError::NotInitialize),
            Some(md) => {
                if line_range.range.is_empty() {
                    return Err(GrabError::InvalidRange {
                        range: line_range.clone(),
                        context: "Cannot get entries of empty range".to_string(),
                    });
                }
                self.source.get_entries(md, line_range)
            }
        }
    }

    /// if the metadata was already created, we know the number of log entries in a file
    pub fn log_entry_count(&self) -> Option<usize> {
        self.metadata.as_ref().map(|md| md.line_count)
    }

    // pub async fn create_metadata_async(
    //     path: &Path,
    // ) -> Result<ComputationResult<GrabMetadata>, GrabError> {
    //     log::trace!("create_metadata_async");
    //     let p = PathBuf::from(path);
    //     let res = tokio::task::spawn_blocking(move || T::from_file(None).unwrap()).await;
    //     let g: ComputationResult<GrabMetadata> =
    //         res.map_err(|e| GrabError::Config(format!("Error executing async grab: {}", e)))?;
    //     Ok(g)
    // }

    pub fn export_slots(
        &self,
        out_path: impl AsRef<Path> + std::fmt::Debug,
    ) -> Result<(), GrabError> {
        let encoded: Vec<u8> = bincode::serialize(&self.metadata)
            .map_err(|_| GrabError::Config("Could not serialize metadata".to_string()))?;
        let mut output = fs::File::create(&out_path)?;
        output.write_all(&encoded)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilePart {
    pub offset_in_file: u64,
    pub length: usize, // how many bytes
    pub total_lines: usize,
    pub lines_to_skip: usize,
    pub lines_to_drop: usize,
}
/// given a list of slots that define byte and line ranges, this function will
/// find the byte range that needs to be read so that all content in lines is captured.
/// It will also return how many lines are in this byte-range and how many need to be skipped
/// at the beginning and dropped the end to get only the desired content
pub(crate) fn identify_byte_range(slots: &[Slot], lines: &LineRange) -> Option<FilePart> {
    trace!("identify byte range for: {:?} (range {:?})", slots, lines);
    if lines.is_empty() {
        return None;
    }
    let start_line_index = lines.start();
    let last_line_index = lines.end();
    let maybe_start_slot = identify_start_slot_simple(slots, start_line_index);
    let maybe_end_slot = identify_end_slot_simple(slots, last_line_index);
    trace!(
        "(maybe_start_slot, maybe_end_slot): ({:?}, {:?})",
        &maybe_start_slot,
        &maybe_end_slot
    );
    match (maybe_start_slot, maybe_end_slot) {
        (Some((start_slot, _)), Some((end_slot, _))) => {
            let lines_to_skip = start_line_index - start_slot.lines.start();
            let lines_to_drop = end_slot.lines.end() - last_line_index;
            let total_lines = end_slot.lines.end() - start_slot.lines.start() + 1;
            let byte_range = ByteRange::from(start_slot.bytes.start()..=end_slot.bytes.end());
            Some(FilePart {
                offset_in_file: byte_range.start(),
                length: byte_range.size() as usize,
                total_lines: total_lines as usize,
                lines_to_skip: lines_to_skip as usize,
                lines_to_drop: lines_to_drop as usize,
            })
        }
        _ => None,
    }
}

pub(crate) fn identify_end_slot_simple(slots: &[Slot], line_index: u64) -> Option<(Slot, usize)> {
    trace!("identify_end_slot_simple");
    match slots.len() {
        0 => None,
        slots_len => {
            let mut i = slots_len;
            for slot in slots.iter().rev() {
                i -= 1;
                if slot.lines.range.contains(&line_index) {
                    return Some((slot.clone(), i));
                }
            }
            None
        }
    }
}

pub(crate) fn identify_start_slot_simple(slots: &[Slot], line_index: u64) -> Option<(Slot, usize)> {
    trace!("identify_start_slot_simple");
    for (i, slot) in slots.iter().enumerate() {
        if slot.lines.range.contains(&line_index) {
            return Some((slot.clone(), i));
        }
    }
    None
}
/// In order to quickly identify the byte index of a line in a logfile,
/// we devide the logfile into slots that store the byte offsets.
/// When we now want to find out the byte offset of a line, we first can quickly
/// identify in which slot it is
#[allow(dead_code)]
pub(crate) fn identify_start_slot(slots: &[Slot], line_index: u64) -> Option<(Slot, usize)> {
    // println!("identify index {}", line_index);
    if slots.is_empty() {
        return None;
    }
    // NOTE: Binary search for slots
    let mut to_investigate = (0, slots.len() - 1);
    loop {
        let slot_mid_index = (to_investigate.0 + to_investigate.1) / 2;
        let slot = &slots[slot_mid_index];
        // println!("examine slot {:?}", slot.lines);
        if (line_index == 0 && slot.lines.start() == 0) || slot.lines.range.contains(&line_index) {
            // println!("found it! 1");
            return Some((slot.clone(), slot_mid_index));
        }
        if to_investigate.1 - to_investigate.0 <= 1 {
            // only 2 possibilities left
            // we already checked slot_mid_index which equals to_investigate.0
            // so check the last possibility
            // let (r, (lower, upper)) = self.slots[to_investigate.1];
            let slot = &slots[to_investigate.1];
            if slot.lines.range.contains(&line_index) {
                // println!("found it! 2");
                return Some((slot.clone(), to_investigate.1));
            }
            break; // not found -> exit
        }
        let old_to_investigate = to_investigate;
        if line_index < slot.lines.start() {
            to_investigate = (to_investigate.0, slot_mid_index);
        } else {
            to_investigate = (slot_mid_index, to_investigate.1);
        }
        if to_investigate == old_to_investigate {
            break;
        }
    }
    None
}
