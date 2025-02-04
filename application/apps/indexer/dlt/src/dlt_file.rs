// Copyright (c) 2020 E.S.R.Labs. All rights reserved.
//
// NOTICE:  All information contained herein is, and remains
// the property of E.S.R.Labs and its suppliers, if any.
// The intellectual and technical concepts contained herein are
// proprietary to E.S.R.Labs and its suppliers and may be covered
// by German and Foreign Patents, patents in process, and are protected
// by trade secret or copyright law.
// Dissemination of this information or reproduction of this material
// is strictly forbidden unless prior written permission is obtained
// from E.S.R.Labs.
use buf_redux::{policy::MinBuffered, BufReader as ReduxReader};
use crossbeam_channel as cc;
use dlt_core::{
    dlt::{LogLevel, Message},
    fibex::{gather_fibex_data, FibexConfig, FibexMetadata},
    filtering,
    fmt::{FormatOptions, FormattableMessage},
    parse::{
        dlt_consume_msg, dlt_message, forward_to_next_storage_header, skip_storage_header,
        DltParseError, ParsedMessage,
    },
    statistics::{
        dlt_statistic_row_info, IdMap, LevelDistribution, StatisticInfo, StatisticRowInfo,
    },
};
use indexer_base::{
    chunks::{ChunkFactory, ChunkResults},
    config::*,
    progress::*,
    utils,
};
use rustc_hash::FxHashMap;
use std::{
    fs,
    io::{BufRead, BufWriter, Read, Seek, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Config problem found: {0}")]
    Config(String),
    #[error("IO error: {0:?}")]
    Io(#[from] std::io::Error),
    #[error("Utils error: {0:?}")]
    Utils(#[from] indexer_base::utils::Error),
}

pub(crate) const STOP_CHECK_LINE_THRESHOLD: usize = 250_000;
pub(crate) const DLT_READER_CAPACITY: usize = 10 * 1024 * 1024;
pub(crate) const DLT_MIN_BUFFER_SPACE: usize = 10 * 1024;
pub(crate) const DLT_PATTERN_SIZE: usize = 4;

pub type StatisticsResults = std::result::Result<IndexingProgress<StatisticInfo>, Notification>;

pub async fn parse_dlt_file(
    in_file: PathBuf,
    filter_config: Option<filtering::ProcessedDltFilterConfig>,
    fibex: Option<FibexConfig>,
) -> Result<Vec<Message>, DltParseError> {
    trace!("parse_dlt_file");
    let fibex_metadata: Option<FibexMetadata> = fibex.map(gather_fibex_data).flatten();
    let source_file_size = fs::metadata(&in_file)?.len();
    let (update_channel, _rx): (cc::Sender<ChunkResults>, cc::Receiver<ChunkResults>) =
        cc::unbounded();
    let mut progress_reporter = ProgressReporter::new(source_file_size, update_channel.clone());
    let mut messages: Vec<Message> = Vec::new();

    let f = match fs::File::open(&in_file) {
        Ok(file) => file,
        Err(e) => {
            warn!("could not open {:?}", in_file);
            let _ = update_channel.try_send(Err(Notification {
                severity: Severity::WARNING,
                content: format!("could not open file ({})", e),
                line: None,
            }));
            return Err(DltParseError::Unrecoverable(format!(
                "could not open file ({})",
                e
            )));
        }
    };
    let mut message_stream = FileMessageProducer::new(f, filter_config, true, fibex_metadata);
    // type Item = Result<Option<Message>, DltParseError>;
    while let Some(msg_result) = tokio_stream::StreamExt::next(&mut message_stream).await {
        trace!("got message from stream: {:?}", msg_result);
        match msg_result {
            Ok((consumed, Some(msg))) => {
                progress_reporter.make_progress(consumed);
                messages.push(msg)
            }
            Ok((consumed, None)) => {
                if consumed == 0 {
                    break;
                }
                progress_reporter.make_progress(consumed);
            }
            Err(e) => warn!("could not produce message: {}", e),
        }
    }
    Ok(messages)
}

pub fn create_index_and_mapping_dlt(
    config: IndexingConfig,
    source_file_size: u64,
    dlt_filter: Option<filtering::DltFilterConfig>,
    update_channel: &cc::Sender<ChunkResults>,
    shutdown_receiver: Option<cc::Receiver<()>>,
    fibex: Option<FibexConfig>,
    fmt_options: Option<FormatOptions>,
) -> Result<(), Error> {
    trace!("create_index_and_mapping_dlt");
    let filter_config: Option<filtering::ProcessedDltFilterConfig> =
        dlt_filter.map(filtering::process_filter_config);
    let fibex_metadata: Option<FibexMetadata> = fibex.map(gather_fibex_data).flatten();

    let f = fs::File::open(&config.in_file)?;
    //     Ok(file) => file,
    //     Err(e) => {
    //         warn!("could not open {:?}", config.in_file);
    //         let _ = update_channel.try_send(Err(Notification {
    //             severity: Severity::WARNING,
    //             content: format!("could not open file ({})", e),
    //             line: None,
    //         }));
    //         return Err(anyhow!(format!("could not open file ({})", e)));
    //     }
    // };
    let mut message_producer = FileMessageProducer::new(f, filter_config, true, fibex_metadata);
    index_dlt_content(
        config,
        source_file_size,
        update_channel,
        shutdown_receiver,
        &mut message_producer,
        fmt_options,
    )
}

#[derive(Debug)]
pub struct MessageStats {
    parsed: usize,
    no_parse: usize,
}
pub struct FileMessageProducer<R>
where
    R: Read + Seek + Unpin,
{
    reader: ReduxReader<R, MinBuffered>,
    filter_config: Option<filtering::ProcessedDltFilterConfig>,
    stats: MessageStats,
    with_storage_header: bool,
    fibex_metadata: Option<FibexMetadata>,
}

impl<R> FileMessageProducer<R>
where
    R: Read + Seek + Unpin,
{
    pub fn new(
        input: R,
        filter_config: Option<filtering::ProcessedDltFilterConfig>,
        with_storage_header: bool,
        fibex_metadata: Option<FibexMetadata>,
    ) -> FileMessageProducer<R> {
        let reader = ReduxReader::with_capacity(DLT_READER_CAPACITY, input)
            .set_policy(MinBuffered(DLT_MIN_BUFFER_SPACE));
        FileMessageProducer {
            reader,
            filter_config,
            stats: MessageStats {
                parsed: 0,
                no_parse: 0,
            },
            with_storage_header,
            fibex_metadata,
        }
    }

    fn fibex(&self) -> Option<&FibexMetadata> {
        self.fibex_metadata.as_ref()
    }

    // Produces the next message in a file or None if the EOF was reached
    fn produce_next_message(&mut self) -> (usize, Result<Option<ParsedMessage>, DltParseError>) {
        #[allow(clippy::never_loop)]
        let consume_and_parse_result: (
            usize,
            Result<Option<ParsedMessage>, DltParseError>,
        ) = loop {
            match self.reader.fill_buf() {
                Ok(content) => {
                    if content.is_empty() {
                        trace!("0, Ok(None)");
                        return (0, Ok(None));
                    }
                    let available = content.len();

                    let parse_result: Result<(&[u8], ParsedMessage), DltParseError> = dlt_message(
                        content,
                        self.filter_config.as_ref(),
                        self.with_storage_header,
                    );

                    match parse_result {
                        Ok((rest, parsed_msg)) => {
                            let consumed = available - rest.len();
                            self.stats.parsed += 1;
                            break (consumed, Ok(Some(parsed_msg)));
                        }
                        Err(DltParseError::IncompleteParse { needed }) => {
                            debug!("parse incomplete");
                            self.stats.no_parse += 1;
                            let needed_s = match needed {
                                Some(s) => format!("{}", s),
                                None => "unknown".to_string(),
                            };
                            break (0, Err(DltParseError::Unrecoverable (
                                format!(
                                    "read_one_dlt_message: imcomplete parsing error for dlt messages: (bytes left: {}, but needed: {})",
                                    content.len(),
                                    needed_s
                                ),
                            )));
                        }
                        Err(DltParseError::ParsingHickup(reason)) => {
                            warn!("parse error");
                            self.stats.no_parse += 1;
                            break (
                                DLT_PATTERN_SIZE,
                                Err(DltParseError::ParsingHickup(format!(
                                    "read_one_dlt_message: parsing error for dlt messages: {}",
                                    reason
                                ))),
                            );
                        }
                        Err(DltParseError::Unrecoverable(cause)) => {
                            warn!("parse failure");
                            self.stats.no_parse += 1;
                            break (
                                0,
                                Err(DltParseError::Unrecoverable(format!(
                                    "read_one_dlt_message: parsing failure for dlt messages: {}",
                                    cause
                                ))),
                            );
                        }
                    }
                }
                Err(e) => {
                    trace!("no more content");
                    break (
                        0,
                        Err(DltParseError::Unrecoverable(format!(
                            "error for filling buffer with dlt messages: {:?}",
                            e
                        ))),
                    );
                }
            }
        };
        self.reader.consume(consume_and_parse_result.0);
        trace!(
            "produce_next_message (consumed {})",
            consume_and_parse_result.0
        );
        consume_and_parse_result
    }
}

impl<R> Iterator for FileMessageProducer<R>
where
    R: Read + Seek + Unpin,
{
    type Item = ParsedMessage;
    fn next(&mut self) -> Option<ParsedMessage> {
        match self.produce_next_message() {
            (_s, Ok(Some(parsed_msg))) => Some(parsed_msg),
            _ => None,
        }
    }
}

impl<R> tokio_stream::Stream for FileMessageProducer<R>
where
    R: Read + Seek + Unpin,
{
    type Item = Result<(usize, Option<Message>), DltParseError>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context,
    ) -> core::task::Poll<Option<Self::Item>> {
        let (consumed, next) = self.produce_next_message();
        match next {
            Ok(Some(ParsedMessage::Item(msg))) => {
                core::task::Poll::Ready(Some(Ok((consumed, Some(msg)))))
            }
            Ok(Some(ParsedMessage::Invalid)) => core::task::Poll::Ready(Some(Ok((consumed, None)))),
            Ok(Some(ParsedMessage::FilteredOut(_))) => {
                core::task::Poll::Ready(Some(Ok((consumed, None))))
            }
            Ok(None) => core::task::Poll::Ready(Some(Ok((consumed, None)))),
            Err(e) => core::task::Poll::Ready(Some(Err(e))),
        }
    }
}

/// count how many recognizable DLT messages are stored in a file
/// each message needs to be equiped with a storage header
pub fn count_dlt_messages(input: &Path) -> Result<u64, DltParseError> {
    if input.exists() {
        let f = fs::File::open(&input)?;

        let mut reader = ReduxReader::with_capacity(DLT_READER_CAPACITY, f)
            .set_policy(MinBuffered(DLT_MIN_BUFFER_SPACE));

        let mut msg_cnt: u64 = 0;
        loop {
            match reader.fill_buf() {
                Ok(content) => {
                    if content.is_empty() {
                        break;
                    }
                    if let Ok((_rest, Some(consumed))) = dlt_consume_msg(content) {
                        reader.consume(consumed as usize);
                        msg_cnt += 1;
                    } else {
                        break;
                    }
                }
                Err(e) => {
                    trace!("no more content");
                    return Err(DltParseError::Unrecoverable(format!(
                        "error for filling buffer with dlt messages: {:?}",
                        e
                    )));
                }
            }
        }
        Ok(msg_cnt)
    } else {
        Err(DltParseError::Unrecoverable(format!(
            "Couldn't find dlt file: {:?}",
            input
        )))
    }
}

/// create index for a dlt file
/// source_file_size: if progress updates should be made, add this value
#[allow(clippy::cognitive_complexity)]
pub fn index_dlt_content<R: Read + Seek + Unpin>(
    config: IndexingConfig,
    source_file_size: u64,
    update_channel: &cc::Sender<ChunkResults>,
    shutdown_receiver: Option<cc::Receiver<()>>,
    message_producer: &mut FileMessageProducer<R>,
    fmt_options: Option<FormatOptions>,
) -> Result<(), Error> {
    trace!("index_dlt_file {:?}", config);
    let (out_file, current_out_file_size) =
        utils::get_out_file_and_size(config.append, &config.out_path)?;

    let mut chunk_count = 0usize;
    let mut last_byte_index = 0usize;
    let mut chunk_factory = ChunkFactory::new(config.chunk_size, current_out_file_size);
    let mut line_nr = if config.append {
        utils::next_line_nr(&config.out_path)?
    } else {
        0
    };
    // let tmp_file = create_dlt_tmp_file("file")?;
    // let mut tmp_writer = BufWriter::with_capacity(DLT_READER_CAPACITY, tmp_file);
    let mut buf_writer = BufWriter::with_capacity(DLT_READER_CAPACITY, out_file);

    let mut progress_reporter = ProgressReporter::new(source_file_size, update_channel.clone());

    let mut stopped = false;
    let mut skipped = 0usize;
    'reading_messages: loop {
        if stopped {
            info!("we were stopped in dlt-indexer",);
            break 'reading_messages;
        };
        let (consumed, next) = message_producer.produce_next_message();
        if consumed == 0 {
            break 'reading_messages;
        } else {
            progress_reporter.make_progress(consumed);
        }
        match next {
            Ok(Some(ParsedMessage::Item(msg))) => {
                trace!("[line:{}] next was Ok(ParsedMessage::Item(msg))", line_nr);
                let formattable_msg = FormattableMessage {
                    message: msg,
                    fibex_metadata: message_producer.fibex(),
                    options: fmt_options.as_ref(),
                };
                let written_bytes_len = utils::create_tagged_line_d(
                    &config.tag,
                    &mut buf_writer,
                    &formattable_msg,
                    line_nr,
                    true,
                )?;
                // tmp_writer.write_all(&msg.as_bytes())?;
                line_nr += 1;
                if let Some(chunk) = chunk_factory.add_bytes(line_nr, written_bytes_len) {
                    stopped = utils::check_if_stop_was_requested(
                        shutdown_receiver.as_ref(),
                        "dlt indexer",
                    );
                    chunk_count += 1;
                    last_byte_index = chunk.b.1;
                    let _ = update_channel.send(Ok(IndexingProgress::GotItem { item: chunk }));
                    buf_writer.flush()?;
                }
            }
            Ok(Some(ParsedMessage::Invalid)) => {
                trace!("next was Ok(ParsedMessage::Invalid)");
            }
            Ok(Some(ParsedMessage::FilteredOut(_))) => {
                trace!("next was Ok(ParsedMessage::FilteredOut)");
                skipped += 1;
            }
            Ok(None) => {
                trace!("next was OK (EOF)");
                break 'reading_messages;
            }
            Err(e) => match e {
                DltParseError::ParsingHickup(reason) => {
                    warn!(
                        "error parsing 1 dlt message, try to continue parsing: {}",
                        reason
                    );
                }
                e => {
                    warn!("error while parsing: {}", e);
                    let _ = update_channel.send(Err(Notification {
                        severity: Severity::ERROR,
                        content: format!("error while parsing dlt file: {}", e),
                        line: None,
                    }));
                    break 'reading_messages;
                }
            },
        }
    }

    trace!("buf_writer.flush()");
    buf_writer.flush()?;
    // tmp_writer.flush()?;
    if let Some(chunk) = chunk_factory.create_last_chunk(line_nr, chunk_count == 0) {
        trace!("send chunk {:?}", chunk);
        let _ = update_channel.send(Ok(IndexingProgress::GotItem {
            item: chunk.clone(),
        }));
        chunk_count += 1;
        last_byte_index = chunk.b.1;
    }
    if chunk_count > 0 {
        let last_expected_byte_index = fs::metadata(config.out_path).map(|md| md.len() as usize)?;
        if last_expected_byte_index != last_byte_index {
            let _ = update_channel.send(Err(Notification {
                severity: Severity::ERROR,
                content: format!(
                    "error in computation! last byte in chunks is {} but should be {}",
                    last_byte_index, last_expected_byte_index
                ),
                line: Some(line_nr),
            }));
        }
    }
    debug!(
        "sending IndexingProgress::Finished (skipped {} msgs)",
        skipped
    );
    let _ = update_channel.send(Ok(IndexingProgress::Finished));
    Ok(())
}

pub fn export_session_file(
    session_id: String,
    destination_path: PathBuf,
    sections: SectionConfig,
    update_channel: cc::Sender<ChunkResults>,
) -> Result<(), Error> {
    trace!(
        "export_as_dlt_file with id: {} to file: {:?}, exporting {:?}",
        session_id,
        destination_path,
        sections
    );
    let session_file_path = session_file_path(&session_id)
        .ok_or_else(|| Error::Config("Session file path unavailable".to_owned()))?;
    export_as_dlt_file(
        session_file_path,
        destination_path,
        sections,
        update_channel,
    )
}

pub fn export_as_dlt_file(
    dlt_file_path: PathBuf,
    destination_path: PathBuf,
    sections: SectionConfig,
    update_channel: cc::Sender<ChunkResults>,
) -> Result<(), Error> {
    trace!(
        "export_as_dlt_file {:?} to file: {:?}, exporting {:?}",
        dlt_file_path,
        destination_path,
        sections
    );
    if dlt_file_path.exists() {
        trace!("found file to export: {:?}", &dlt_file_path);
        let f = fs::File::open(&dlt_file_path)?;
        let mut reader = &mut std::io::BufReader::new(f);
        let out_file = std::fs::File::create(destination_path)?;
        trace!("created out_file: {:?}", &out_file);
        let partitioner = FilePartitioner::new(&dlt_file_path, sections)?;
        let mut out_writer = BufWriter::new(out_file);

        for part in partitioner.get_parts() {
            trace!("copy part {:?}", part);
            reader.seek(std::io::SeekFrom::Start(part.offset as u64))?;
            let mut take = reader.take(part.length as u64);
            std::io::copy(&mut take, &mut out_writer)?;
            reader = take.into_inner();
            out_writer.flush()?;
        }
        let _ = update_channel.send(Ok(IndexingProgress::Finished));
        Ok(())
    } else {
        let reason = format!("couln't find session file: {:?}", dlt_file_path,);
        let _ = update_channel.send(Err(Notification {
            severity: Severity::ERROR,
            content: reason.clone(),
            line: None,
        }));
        Err(Error::Config(reason))
    }
}

pub(crate) fn session_file_path(session_id: &str) -> Option<PathBuf> {
    let home_dir = dirs::home_dir()?;
    let tmp_file_name = format!("{}.dlt", session_id);
    Some(
        home_dir
            .join(".chipmunk")
            .join("streams")
            .join(tmp_file_name),
    )
}

pub(crate) fn create_dlt_session_file(session_id: &str) -> Option<std::fs::File> {
    let path = session_file_path(session_id)?;
    std::fs::File::create(path).ok()
}

struct FilePartitioner {
    reader: ReduxReader<fs::File, MinBuffered>,
    offset: u64,
    section_config: SectionConfig,
    file_size: u64,
}
impl FilePartitioner {
    fn new(in_path: &Path, c: SectionConfig) -> Result<Self, std::io::Error> {
        let f = fs::File::open(&in_path)?;
        Ok(FilePartitioner {
            reader: ReduxReader::with_capacity(DLT_READER_CAPACITY, f)
                .set_policy(MinBuffered(DLT_MIN_BUFFER_SPACE)),
            offset: 0,
            section_config: c,
            file_size: fs::metadata(in_path)?.len(),
        })
    }
    fn get_parts(mut self) -> Vec<FilePart> {
        if self.section_config.sections.is_empty() {
            return vec![FilePart {
                offset: 0,
                length: self.file_size,
            }];
        }
        #[derive(Debug)]
        struct State {
            index: usize,
            in_section: bool,
            bytes_in_section: u64,
            section_offset: u64,
        }
        let mut result_vec: Vec<FilePart> = vec![];
        let mut state = State {
            index: 0,
            in_section: false,
            bytes_in_section: 0,
            section_offset: 0,
        };
        impl State {
            fn reset_section(&mut self) {
                self.in_section = false;
                self.bytes_in_section = 0;
            }
        }
        for section in self.section_config.sections {
            loop {
                trace!("next[{:?}] (current section {:?})", state.index, section);
                match self.reader.fill_buf() {
                    Ok(content) => {
                        trace!("Ok(content (len {}))", content.len());
                        if content.is_empty() {
                            trace!("0, Ok(ParsedMessage::Invalid)");
                            break;
                        }
                        match skip_storage_header(content) {
                            Ok((rest, skipped_bytes)) => {
                                let (len_without_storage_header, _was_last) =
                                    match forward_to_next_storage_header(rest) {
                                        Some((dropped, _)) => (dropped, false),
                                        None => (rest.len() as u64, true),
                                    };
                                let consumed = skipped_bytes + len_without_storage_header;
                                if state.index == section.first_line {
                                    trace!("---> enter section: {:?}) ({:?})", section, state);
                                    state.in_section = true;
                                    state.section_offset = self.offset;
                                }
                                if state.in_section {
                                    state.bytes_in_section += consumed as u64;
                                }
                                if state.index == section.last_line {
                                    trace!("<--- leaving section: {:?}) ({:?})", section, state);
                                    let res = FilePart {
                                        offset: state.section_offset,
                                        length: state.bytes_in_section,
                                    };
                                    trace!(
                                        "consumed: {}bytes, res: {:?}",
                                        state.bytes_in_section,
                                        &res
                                    );
                                    result_vec.push(res);
                                    state.reset_section();
                                    break;
                                }
                                self.offset += consumed;
                                self.reader.consume(consumed as usize);
                            }
                            Err(_e) => {
                                warn!("error in FilePartitioner forward: {}", _e);
                                break;
                            }
                        }
                    }
                    Err(_e) => {
                        warn!("error in FilePartitioner iterator: {}", _e);
                        break;
                    }
                }
                state.index += 1;
            }
        }
        trace!("result of partition: {:?}", result_vec);
        result_vec
    }
}
#[derive(Debug)]
struct FilePart {
    offset: u64,
    length: u64,
}

pub fn get_dlt_file_info(
    in_file: &Path,
    update_channel: &cc::Sender<StatisticsResults>,
    shutdown_receiver: Option<crossbeam_channel::Receiver<()>>,
) -> Result<(), DltParseError> {
    let f = fs::File::open(in_file)?;

    let source_file_size = fs::metadata(&in_file)?.len();
    let mut reader = ReduxReader::with_capacity(DLT_READER_CAPACITY, f)
        .set_policy(MinBuffered(DLT_MIN_BUFFER_SPACE));

    let mut app_ids: IdMap = FxHashMap::default();
    let mut context_ids: IdMap = FxHashMap::default();
    let mut ecu_ids: IdMap = FxHashMap::default();
    let mut index = 0usize;
    let mut processed_bytes = 0u64;
    let mut contained_non_verbose = false;
    loop {
        match read_one_dlt_message_info(&mut reader, true) {
            Ok(Some((
                consumed,
                StatisticRowInfo {
                    app_id_context_id: Some((app_id, context_id)),
                    ecu_id: ecu,
                    level,
                    verbose,
                },
            ))) => {
                contained_non_verbose = contained_non_verbose || !verbose;
                reader.consume(consumed as usize);
                add_for_level(level, &mut app_ids, app_id);
                add_for_level(level, &mut context_ids, context_id);
                match ecu {
                    Some(id) => add_for_level(level, &mut ecu_ids, id),
                    None => add_for_level(level, &mut ecu_ids, "NONE".to_string()),
                };
                processed_bytes += consumed;
            }
            Ok(Some((
                consumed,
                StatisticRowInfo {
                    app_id_context_id: None,
                    ecu_id: ecu,
                    level,
                    verbose,
                },
            ))) => {
                contained_non_verbose = contained_non_verbose || !verbose;
                reader.consume(consumed as usize);
                add_for_level(level, &mut app_ids, "NONE".to_string());
                add_for_level(level, &mut context_ids, "NONE".to_string());
                match ecu {
                    Some(id) => add_for_level(level, &mut ecu_ids, id),
                    None => add_for_level(level, &mut ecu_ids, "NONE".to_string()),
                };
                processed_bytes += consumed;
            }
            Ok(None) => {
                break;
            }
            // Err(e) => {
            //     return Err(err_msg(format!(
            //         "error while parsing dlt messages[{}]: {}",
            //         index, e
            //     )))
            Err(e) => {
                // we couldn't parse the message. try to skip it and find the next.
                debug!("stats...try to skip and continue parsing: {}", e);
                match e {
                    DltParseError::ParsingHickup(reason) => {
                        // we couldn't parse the message. try to skip it and find the next.
                        reader.consume(4); // at least skip the magic DLT pattern
                        debug!(
                            "error parsing 1 dlt message, try to continue parsing: {}",
                            reason
                        );
                    }
                    DltParseError::Unrecoverable(cause) => {
                        warn!("cannot continue parsing: {}", cause);
                        let _ = update_channel.send(Err(Notification {
                            severity: Severity::ERROR,
                            content: format!("error parsing dlt file: {}", cause),
                            line: None,
                        }));
                        break;
                    }
                    DltParseError::IncompleteParse { needed } => {
                        warn!(
                            "cannot continue parsing, parse was incomplete: {:?}",
                            needed
                        );
                        let _ = update_channel.send(Err(Notification {
                            severity: Severity::ERROR,
                            content: format!("parse was incomplete: {:?}", needed),
                            line: None,
                        }));
                        break;
                    }
                }
            }
        }
        index += 1;
        if index % STOP_CHECK_LINE_THRESHOLD == 0 {
            if utils::check_if_stop_was_requested(shutdown_receiver.as_ref(), "dlt stats producer")
            {
                let _ = update_channel.send(Ok(IndexingProgress::Stopped));
                break;
            }
            let _ = update_channel.send(Ok(IndexingProgress::Progress {
                ticks: (processed_bytes, source_file_size),
            }));
        }
    }
    let res = StatisticInfo {
        app_ids: app_ids
            .into_iter()
            .collect::<Vec<(String, LevelDistribution)>>(),
        context_ids: context_ids
            .into_iter()
            .collect::<Vec<(String, LevelDistribution)>>(),
        ecu_ids: ecu_ids
            .into_iter()
            .collect::<Vec<(String, LevelDistribution)>>(),
        contained_non_verbose,
    };

    let _ = update_channel.send(Ok(IndexingProgress::GotItem { item: res }));
    let _ = update_channel.send(Ok(IndexingProgress::Finished));
    Ok(())
}

fn add_for_level(level: Option<LogLevel>, ids: &mut IdMap, id: String) {
    if let Some(n) = ids.get_mut(&id) {
        match level {
            Some(LogLevel::Fatal) => {
                *n = LevelDistribution {
                    log_fatal: n.log_fatal + 1,
                    ..*n
                }
            }
            Some(LogLevel::Error) => {
                *n = LevelDistribution {
                    log_error: n.log_error + 1,
                    ..*n
                }
            }
            Some(LogLevel::Warn) => {
                *n = LevelDistribution {
                    log_warning: n.log_warning + 1,
                    ..*n
                }
            }
            Some(LogLevel::Info) => {
                *n = LevelDistribution {
                    log_info: n.log_info + 1,
                    ..*n
                }
            }
            Some(LogLevel::Debug) => {
                *n = LevelDistribution {
                    log_debug: n.log_debug + 1,
                    ..*n
                };
            }
            Some(LogLevel::Verbose) => {
                *n = LevelDistribution {
                    log_verbose: n.log_verbose + 1,
                    ..*n
                };
            }
            Some(LogLevel::Invalid(_)) => {
                *n = LevelDistribution {
                    log_invalid: n.log_invalid + 1,
                    ..*n
                };
            }
            None => {
                *n = LevelDistribution {
                    non_log: n.non_log + 1,
                    ..*n
                };
            }
        }
    } else {
        ids.insert(id, LevelDistribution::new(level));
    }
}

fn read_one_dlt_message_info<T: Read>(
    reader: &mut ReduxReader<T, MinBuffered>,
    with_storage_header: bool,
) -> Result<Option<(u64, StatisticRowInfo)>, DltParseError> {
    match reader.fill_buf() {
        Ok(content) => {
            if content.is_empty() {
                return Ok(None);
            }
            let available = content.len();
            let r = dlt_statistic_row_info(content, with_storage_header)?;
            let consumed = available - r.0.len();
            Ok(Some((consumed as u64, r.1)))
        }
        Err(e) => Err(DltParseError::ParsingHickup(format!(
            "error while parsing dlt messages: {}",
            e
        ))),
    }
}
