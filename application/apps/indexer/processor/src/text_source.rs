use crate::grabber::{
    identify_byte_range, ByteRange, GrabError, GrabMetadata, GrabbedContent, GrabbedElement,
    LineRange, MetadataSource, Slot,
};
use buf_redux::{policy::MinBuffered, BufReader as ReduxReader};
use indexer_base::{progress::ComputationResult, utils};
use std::{
    fs,
    io::{BufRead, Read, SeekFrom},
    path::{Path, PathBuf},
};

const REDUX_READER_CAPACITY: usize = 1024 * 1024;
const REDUX_MIN_BUFFER_SPACE: usize = 10 * 1024;

#[derive(Debug)]
pub struct TextFileSource {
    source_id: String,
    path: PathBuf,
}

impl TextFileSource {
    pub fn new(p: &Path, id: &str) -> Self {
        Self {
            source_id: id.to_string(),
            path: PathBuf::from(p),
        }
    }
}

impl MetadataSource for TextFileSource {
    fn source_id(&self) -> String {
        self.source_id.clone()
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn count_lines(&self) -> Result<usize, GrabError> {
        let chunk_size = 100 * 1024usize;
        let mut f = fs::File::open(&self.path)?;
        let mut count = 0usize;
        let mut buffer = vec![0; chunk_size];

        loop {
            let n = f.read(&mut buffer)?;
            if n < chunk_size {
                buffer.resize(n, 0);
            }
            if n == 0 {
                break;
            }
            count += bytecount::count(&buffer, b'\n');
            if n < chunk_size {
                break;
            }
        }
        Ok(count)
    }

    fn from_file(
        &self,
        shutdown_receiver: Option<cc::Receiver<()>>,
    ) -> Result<ComputationResult<GrabMetadata>, GrabError> {
        let f = fs::File::open(&self.path)?;
        let mut slots = Vec::<Slot>::new();
        let mut byte_index = 0u64;
        let mut line_index = 0u64;

        let mut reader = ReduxReader::with_capacity(REDUX_READER_CAPACITY, f)
            .set_policy(MinBuffered(REDUX_MIN_BUFFER_SPACE));

        loop {
            if utils::check_if_stop_was_requested(shutdown_receiver.as_ref(), "grabber") {
                return Ok(ComputationResult::Stopped);
            }
            match reader.fill_buf() {
                Ok(content) => {
                    if content.is_empty() {
                        // everything was processed
                        break;
                    }

                    let (nl, offset_last_newline) = count_lines_up_to_last_newline(content);
                    let (slot, consumed, processed_lines) = if nl == 0 {
                        let consumed = content.len() as u64;
                        // we hit a very long line that exceeds our read buffer, best
                        // to package everything we read into an entry and start a new one
                        let slot = Slot {
                            bytes: ByteRange::from(byte_index..=(byte_index + consumed) - 1),
                            lines: LineRange::from(line_index..=line_index),
                        };
                        (slot, consumed, 1)
                    } else {
                        let consumed = offset_last_newline as u64 + 1;
                        let slot = Slot {
                            bytes: ByteRange::from(byte_index..=(byte_index + consumed - 1)),
                            lines: LineRange::from(line_index..=(line_index + nl) - 1),
                        };
                        (slot, consumed, nl)
                    };
                    reader.consume(consumed as usize);
                    slots.push(slot);
                    byte_index += consumed;
                    line_index += processed_lines;
                }
                Err(e) => {
                    trace!("no more content");
                    return Err(GrabError::Config(format!(
                        "error for filling buffer with more content: {:?}",
                        e
                    )));
                }
            }
        }

        Ok(ComputationResult::Item(GrabMetadata {
            slots,
            line_count: line_index as usize,
        }))
    }

    /// Get all lines in a file within the supplied line-range
    /// naive implementation that just reads all slots that are involved and drops
    /// everything that is not needed
    fn get_entries(
        &self,
        metadata: &GrabMetadata,
        line_range: &LineRange,
    ) -> Result<GrabbedContent, GrabError> {
        if line_range.range.is_empty() {
            return Err(GrabError::InvalidRange {
                range: line_range.clone(),
                context: "Get entries of empty range is invalid".to_string(),
            });
        }
        use std::io::prelude::*;
        let file_part = identify_byte_range(&metadata.slots, line_range).ok_or_else(|| {
            GrabError::InvalidRange {
                range: line_range.clone(),
                context: format!("Error identifying byte range for range {:?}", line_range),
            }
        })?;
        // println!(
        //     "relevant file-part (starts at index {}): lines {}",
        //     file_part.offset_in_file,
        //     file_part.total_lines - file_part.lines_to_skip - file_part.lines_to_drop
        // );

        let mut read_buf = vec![0; file_part.length];
        let mut read_from = fs::File::open(&self.path())?;
        read_from.seek(SeekFrom::Start(file_part.offset_in_file))?;

        read_from.read_exact(&mut read_buf)?;

        let s = unsafe { std::str::from_utf8_unchecked(&read_buf) };

        let all_lines = s.split(|c| c == '\n');
        let lines_minus_end = all_lines.take(file_part.total_lines - file_part.lines_to_drop);
        let pure_lines = lines_minus_end.skip(file_part.lines_to_skip);
        let grabbed_elements = pure_lines
            .map(|s| GrabbedElement {
                source_id: self.source_id.clone(),
                content: s.to_owned(),
                row: None,
                pos: None,
            })
            .collect::<Vec<GrabbedElement>>();

        Ok(GrabbedContent { grabbed_elements })
    }
}

fn count_lines_up_to_last_newline(buffer: &[u8]) -> (u64, usize) {
    if let Some(offset) = buffer.iter().rposition(|&v| v == b'\n') {
        (bytecount::count(buffer, b'\n') as u64, offset)
    } else {
        (0, 0)
    }
}
