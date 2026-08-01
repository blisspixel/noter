//! Stable custom benchmark worker for reproducible M1 evidence.

use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use noter::core::document::Document;
use noter::core::save::{Durability, SaveOutcome};
use noter::core::search::{LiteralSearch, MatchCase, SearchDirection};

const SEARCH_EARLY: &str = "NOTER-SEARCH-EARLY-7CDA1B9F";
const SEARCH_MIDDLE: &str = "NOTER-SEARCH-MIDDLE-932E8A04";
const SEARCH_LATE: &str = "NOTER-SEARCH-LATE-45F016BC";
const SEARCH_ABSENT: &str = "NOTER-SEARCH-ABSENT-A217D605";
const ADVERSARIAL_QUERY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab";

#[derive(Clone, Copy, Debug)]
enum Case {
    LoadEmpty,
    LoadProse,
    LoadMixed,
    LoadNewline,
    LoadLongLine,
    LoadSourceLarge,
    LoadLogLarge,
    SearchEarly,
    SearchMiddle,
    SearchLate,
    SearchAbsent,
    SearchAdversarial,
    SerializeProse,
    SaveNewProse,
    SaveReplaceProse,
}

impl Case {
    const ALL: [Self; 15] = [
        Self::LoadEmpty,
        Self::LoadProse,
        Self::LoadMixed,
        Self::LoadNewline,
        Self::LoadLongLine,
        Self::LoadSourceLarge,
        Self::LoadLogLarge,
        Self::SearchEarly,
        Self::SearchMiddle,
        Self::SearchLate,
        Self::SearchAbsent,
        Self::SearchAdversarial,
        Self::SerializeProse,
        Self::SaveNewProse,
        Self::SaveReplaceProse,
    ];

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "load-empty" => Ok(Self::LoadEmpty),
            "load-prose-1mib" => Ok(Self::LoadProse),
            "load-mixed-unicode-eol-1mib" => Ok(Self::LoadMixed),
            "load-newline-1mib" => Ok(Self::LoadNewline),
            "load-long-line-1mib" => Ok(Self::LoadLongLine),
            "load-source-50mib" => Ok(Self::LoadSourceLarge),
            "load-log-50mib" => Ok(Self::LoadLogLarge),
            "search-early-50mib" => Ok(Self::SearchEarly),
            "search-middle-50mib" => Ok(Self::SearchMiddle),
            "search-late-50mib" => Ok(Self::SearchLate),
            "search-absent-50mib" => Ok(Self::SearchAbsent),
            "search-adversarial-50mib" => Ok(Self::SearchAdversarial),
            "serialize-prose-1mib" => Ok(Self::SerializeProse),
            "save-new-prose-1mib" => Ok(Self::SaveNewProse),
            "save-replace-prose-1mib" => Ok(Self::SaveReplaceProse),
            _ => Err(format!("unknown benchmark case: {value}")),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::LoadEmpty => "load-empty",
            Self::LoadProse => "load-prose-1mib",
            Self::LoadMixed => "load-mixed-unicode-eol-1mib",
            Self::LoadNewline => "load-newline-1mib",
            Self::LoadLongLine => "load-long-line-1mib",
            Self::LoadSourceLarge => "load-source-50mib",
            Self::LoadLogLarge => "load-log-50mib",
            Self::SearchEarly => "search-early-50mib",
            Self::SearchMiddle => "search-middle-50mib",
            Self::SearchLate => "search-late-50mib",
            Self::SearchAbsent => "search-absent-50mib",
            Self::SearchAdversarial => "search-adversarial-50mib",
            Self::SerializeProse => "serialize-prose-1mib",
            Self::SaveNewProse => "save-new-prose-1mib",
            Self::SaveReplaceProse => "save-replace-prose-1mib",
        }
    }

    const fn corpus_name(self) -> &'static str {
        match self {
            Self::LoadEmpty => "empty.txt",
            Self::LoadProse
            | Self::SerializeProse
            | Self::SaveNewProse
            | Self::SaveReplaceProse => "prose-1mib.txt",
            Self::LoadMixed => "mixed-unicode-eol-1mib.txt",
            Self::LoadNewline => "newline-1mib.txt",
            Self::LoadLongLine => "long-line-1mib.txt",
            Self::LoadSourceLarge
            | Self::SearchEarly
            | Self::SearchMiddle
            | Self::SearchLate
            | Self::SearchAbsent => "source-large.txt",
            Self::LoadLogLarge | Self::SearchAdversarial => "log-large.txt",
        }
    }
}

#[derive(Debug)]
struct Arguments {
    case: Case,
    corpus_dir: PathBuf,
    work_dir: PathBuf,
    samples: usize,
    warmup: usize,
    hold: bool,
}

enum Action {
    Probe,
    List,
    Run(Arguments),
}

fn take_value(arguments: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        return Err(format!("{flag} may be supplied only once"));
    }
    Ok(())
}

fn parse_count(value: &str, flag: &str, allow_zero: bool) -> Result<usize, String> {
    let count = value
        .parse::<usize>()
        .map_err(|_| format!("{flag} must be a non-negative integer"))?;
    if (!allow_zero && count == 0) || count > 10_000 {
        return Err(format!("{flag} is outside its supported range"));
    }
    Ok(count)
}

fn parse_arguments() -> Result<Action, String> {
    let mut arguments = env::args().skip(1);
    let Some(first) = arguments.next() else {
        return Ok(Action::Probe);
    };
    if first == "--list" {
        if arguments.next().is_some() {
            return Err("--list does not accept additional arguments".to_owned());
        }
        return Ok(Action::List);
    }

    let mut pending = Some(first);
    let mut case = None;
    let mut corpus_dir = None;
    let mut work_dir = None;
    let mut samples = None;
    let mut warmup = None;
    let mut hold = false;
    loop {
        let flag = pending.take().or_else(|| arguments.next());
        let Some(flag) = flag else { break };
        match flag.as_str() {
            "--case" => {
                let value = take_value(&mut arguments, "--case")?;
                set_once(&mut case, Case::parse(&value)?, "--case")?;
            }
            "--corpus-dir" => {
                let value = take_value(&mut arguments, "--corpus-dir")?;
                set_once(&mut corpus_dir, PathBuf::from(value), "--corpus-dir")?;
            }
            "--work-dir" => {
                let value = take_value(&mut arguments, "--work-dir")?;
                set_once(&mut work_dir, PathBuf::from(value), "--work-dir")?;
            }
            "--samples" => {
                let value = take_value(&mut arguments, "--samples")?;
                set_once(
                    &mut samples,
                    parse_count(&value, "--samples", false)?,
                    "--samples",
                )?;
            }
            "--warmup" => {
                let value = take_value(&mut arguments, "--warmup")?;
                set_once(
                    &mut warmup,
                    parse_count(&value, "--warmup", true)?,
                    "--warmup",
                )?;
            }
            "--hold" if !hold => hold = true,
            "--hold" => return Err("--hold may be supplied only once".to_owned()),
            _ => return Err(format!("unknown benchmark argument: {flag}")),
        }
    }

    Ok(Action::Run(Arguments {
        case: case.ok_or_else(|| "--case is required".to_owned())?,
        corpus_dir: corpus_dir.ok_or_else(|| "--corpus-dir is required".to_owned())?,
        work_dir: work_dir.ok_or_else(|| "--work-dir is required".to_owned())?,
        samples: samples.ok_or_else(|| "--samples is required".to_owned())?,
        warmup: warmup.ok_or_else(|| "--warmup is required".to_owned())?,
        hold,
    }))
}

fn emit_case_list() -> Result<(), String> {
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    output
        .write_all(b"[")
        .map_err(|error| write_error(&error))?;
    for (index, case) in Case::ALL.into_iter().enumerate() {
        if index != 0 {
            output
                .write_all(b",")
                .map_err(|error| write_error(&error))?;
        }
        write!(output, "\"{}\"", case.name()).map_err(|error| write_error(&error))?;
    }
    output
        .write_all(b"]\n")
        .map_err(|error| write_error(&error))?;
    output.flush().map_err(|error| write_error(&error))
}

fn probe_case_contract() -> Result<(), String> {
    for (index, case) in Case::ALL.into_iter().enumerate() {
        if case.name().is_empty()
            || !Path::new(case.corpus_name())
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        {
            return Err("benchmark case metadata is incomplete".to_owned());
        }
        if Case::ALL[index + 1..]
            .iter()
            .any(|other| other.name() == case.name())
        {
            return Err("benchmark case names must be unique".to_owned());
        }
    }
    if validated_search_checksum(Some(4), Some(4))? != 5
        || validated_search_checksum(None, None)? != 0
        || validated_search_checksum(Some(4), None).is_ok()
        || validated_search_checksum(None, Some(4)).is_ok()
    {
        return Err("benchmark search-result guard is incomplete".to_owned());
    }
    Ok(())
}

fn write_error(error: &io::Error) -> String {
    format!("write benchmark output failed: {error}")
}

fn measure_validated<T>(
    warmup: usize,
    sample_count: usize,
    mut operation: impl FnMut() -> Result<T, String>,
    mut validate: impl FnMut(T) -> Result<u64, String>,
) -> Result<(Vec<u128>, u64), String> {
    let mut checksum = 0_u64;
    for _ in 0..warmup {
        let result = black_box(operation()?);
        checksum = checksum.rotate_left(1) ^ validate(result)?;
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(sample_count)
        .map_err(|_| "could not allocate raw benchmark samples".to_owned())?;
    for _ in 0..sample_count {
        let started = Instant::now();
        let result = black_box(operation()?);
        let elapsed = started.elapsed().as_nanos().max(1);
        checksum = checksum.rotate_left(1) ^ validate(result)?;
        samples.push(elapsed);
    }
    Ok((samples, checksum))
}

fn measure(
    warmup: usize,
    sample_count: usize,
    operation: impl FnMut() -> Result<u64, String>,
) -> Result<(Vec<u128>, u64), String> {
    measure_validated(warmup, sample_count, operation, Ok)
}

fn corpus_path(arguments: &Arguments) -> Result<PathBuf, String> {
    let name = arguments.case.corpus_name();
    let path = arguments.corpus_dir.join(name);
    if !path.is_file() {
        return Err(format!("required corpus file is missing: {name}"));
    }
    Ok(path)
}

fn run_load(arguments: &Arguments) -> Result<(Vec<u128>, u64), String> {
    let path = corpus_path(arguments)?;
    let mut retained = None;
    let measured = measure(arguments.warmup, arguments.samples, || {
        let document =
            Document::from_path(&path).map_err(|error| format!("load failed: {error}"))?;
        let length = u64::try_from(document.rope().len_bytes())
            .map_err(|_| "document length does not fit the benchmark checksum".to_owned())?;
        retained = Some(document);
        Ok(length)
    })?;
    black_box(&retained);
    Ok(measured)
}

fn search_query(case: Case) -> Result<&'static str, String> {
    match case {
        Case::SearchEarly => Ok(SEARCH_EARLY),
        Case::SearchMiddle => Ok(SEARCH_MIDDLE),
        Case::SearchLate => Ok(SEARCH_LATE),
        Case::SearchAbsent => Ok(SEARCH_ABSENT),
        Case::SearchAdversarial => Ok(ADVERSARIAL_QUERY),
        _ => Err("non-search case requested a query".to_owned()),
    }
}

fn expected_search_offset(case: Case, source_length: usize) -> Result<Option<usize>, String> {
    match case {
        Case::SearchEarly => Ok(Some(4096.min(source_length / 8))),
        Case::SearchMiddle => Ok(Some(source_length / 2)),
        Case::SearchLate => source_length
            .checked_sub(4096)
            .map(Some)
            .ok_or_else(|| "late-search corpus is too short".to_owned()),
        Case::SearchAbsent | Case::SearchAdversarial => Ok(None),
        _ => Err("non-search case requested an expected offset".to_owned()),
    }
}

fn validated_search_checksum(
    expected: Option<usize>,
    actual: Option<usize>,
) -> Result<u64, String> {
    if actual != expected {
        return Err(format!(
            "search result differed from the corpus contract: expected {expected:?}, got {actual:?}"
        ));
    }
    actual.map_or(Ok(0), |offset| {
        u64::try_from(offset)
            .map_err(|_| "search offset does not fit the benchmark checksum".to_owned())?
            .checked_add(1)
            .ok_or_else(|| "search checksum overflowed".to_owned())
    })
}

fn run_search(arguments: &Arguments) -> Result<(Vec<u128>, u64), String> {
    let path = corpus_path(arguments)?;
    let source =
        fs::read_to_string(path).map_err(|error| format!("read corpus failed: {error}"))?;
    let search = LiteralSearch::new(search_query(arguments.case)?, MatchCase::Sensitive)
        .map_err(|error| format!("prepare search failed: {error}"))?;
    let expected = expected_search_offset(arguments.case, source.len())?;
    let measured = measure_validated(
        arguments.warmup,
        arguments.samples,
        || Ok(search.navigate(black_box(&source), 0, SearchDirection::Next)),
        |navigation| {
            validated_search_checksum(expected, navigation.map(|found| found.range().start()))
        },
    )?;
    black_box((&source, &search));
    Ok(measured)
}

fn run_serialize(arguments: &Arguments) -> Result<(Vec<u128>, u64), String> {
    let document = Document::from_path(corpus_path(arguments)?)
        .map_err(|error| format!("load serialization corpus failed: {error}"))?;
    let mut retained = Vec::new();
    let measured = measure(arguments.warmup, arguments.samples, || {
        retained = document.to_bytes();
        u64::try_from(retained.len())
            .map_err(|_| "serialized length does not fit the benchmark checksum".to_owned())
    })?;
    black_box(&retained);
    Ok(measured)
}

fn committed_checksum(outcome: SaveOutcome) -> Result<u64, String> {
    let SaveOutcome::Committed {
        durability,
        observation,
        warnings,
        ..
    } = outcome
    else {
        return Err(format!("save benchmark did not commit: {outcome:?}"));
    };
    if !warnings.is_empty() {
        return Err(format!(
            "save benchmark committed with warnings: {warnings:?}"
        ));
    }
    let durability_value = match durability {
        Durability::FileAndDirectorySynced => 3_u64,
        Durability::FileSynced => 2,
        Durability::BestEffort => 1,
    };
    Ok(observation.length() ^ durability_value)
}

fn create_new_target(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create replacement target failed: {error}"))?;
    target
        .write_all(bytes)
        .map_err(|error| format!("prepare replacement target failed: {error}"))?;
    target
        .flush()
        .map_err(|error| format!("flush replacement target failed: {error}"))
}

fn verify_file_contents(path: &Path, expected: &[u8]) -> Result<(), String> {
    let mut committed = fs::File::open(path)
        .map_err(|error| format!("open committed benchmark destination failed: {error}"))?;
    let mut buffer = [0_u8; 8 * 1024];
    let mut offset = 0;
    while offset < expected.len() {
        let count = (expected.len() - offset).min(buffer.len());
        committed
            .read_exact(&mut buffer[..count])
            .map_err(|error| format!("read committed benchmark destination failed: {error}"))?;
        if buffer[..count] != expected[offset..offset + count] {
            return Err("committed benchmark destination bytes differ".to_owned());
        }
        offset += count;
    }
    let mut trailing = [0_u8; 1];
    if committed
        .read(&mut trailing)
        .map_err(|error| format!("finish committed benchmark verification failed: {error}"))?
        != 0
    {
        return Err("committed benchmark destination has trailing bytes".to_owned());
    }
    Ok(())
}

fn timed_save_samples(
    arguments: &Arguments,
    replace: bool,
) -> Result<(Vec<u128>, u64, Option<Document>), String> {
    let bytes = fs::read(corpus_path(arguments)?)
        .map_err(|error| format!("read save corpus failed: {error}"))?;
    let total = arguments
        .warmup
        .checked_add(arguments.samples)
        .ok_or_else(|| "benchmark iteration count overflowed".to_owned())?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(arguments.samples)
        .map_err(|_| "could not allocate raw benchmark samples".to_owned())?;
    let mut checksum = 0_u64;
    let mut retained = None;
    let replacement_text = if replace {
        let mut replacement = String::from_utf8(bytes.clone())
            .map_err(|_| "save corpus is not strict UTF-8".to_owned())?;
        replacement.replace_range(0..1, "n");
        Some(replacement)
    } else {
        None
    };
    for index in 0..total {
        let target = arguments.work_dir.join(format!("save-{index:05}.txt"));
        let mut document = if replace {
            create_new_target(&target, &bytes)?;
            let mut document = Document::from_path(&target)
                .map_err(|error| format!("load replacement target failed: {error}"))?;
            let replacement = replacement_text
                .as_deref()
                .ok_or_else(|| "replacement benchmark text is missing".to_owned())?;
            document
                .replace_text(replacement)
                .map_err(|error| format!("prepare replacement edit failed: {error}"))?;
            document
        } else {
            Document::from_bytes(&bytes)
                .map_err(|error| format!("prepare new-file document failed: {error}"))?
        };

        let started = Instant::now();
        let outcome = if replace {
            document.save()
        } else {
            document.save_as(&target)
        }
        .map_err(|error| format!("save setup failed: {error}"))?;
        let result = committed_checksum(outcome)?;
        let elapsed = started.elapsed().as_nanos().max(1);
        checksum = checksum.rotate_left(1) ^ result;
        let expected = replacement_text
            .as_deref()
            .map_or(bytes.as_slice(), str::as_bytes);
        verify_file_contents(&target, expected)?;
        if index >= arguments.warmup {
            samples.push(elapsed);
        }
        fs::remove_file(&target)
            .map_err(|error| format!("remove benchmark destination failed: {error}"))?;
        retained = Some(document);
    }
    Ok((samples, checksum, retained))
}

fn run_save(arguments: &Arguments, replace: bool) -> Result<(Vec<u128>, u64), String> {
    let (samples, checksum, retained) = timed_save_samples(arguments, replace)?;
    black_box(&retained);
    Ok((samples, checksum))
}

fn hold_if_requested(hold: bool) -> Result<(), String> {
    if hold {
        let mut release = [0_u8; 1];
        io::stdin()
            .read(&mut release)
            .map_err(|error| format!("read evidence release failed: {error}"))?;
    }
    Ok(())
}

fn emit_result(case: Case, warmup: usize, samples: &[u128], checksum: u64) -> Result<(), String> {
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    write!(
        output,
        "{{\"case\":\"{}\",\"warmup\":{warmup},\"samples_ns\":[",
        case.name()
    )
    .map_err(|error| write_error(&error))?;
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            output
                .write_all(b",")
                .map_err(|error| write_error(&error))?;
        }
        write!(output, "{sample}").map_err(|error| write_error(&error))?;
    }
    writeln!(output, "],\"checksum\":{checksum}}}").map_err(|error| write_error(&error))?;
    output.flush().map_err(|error| write_error(&error))
}

fn run(arguments: &Arguments) -> Result<(), String> {
    if !arguments.corpus_dir.is_dir() {
        return Err("--corpus-dir must name an existing directory".to_owned());
    }
    if !arguments.work_dir.is_dir() {
        return Err("--work-dir must name a newly created directory".to_owned());
    }
    let work_metadata = fs::symlink_metadata(&arguments.work_dir)
        .map_err(|error| format!("inspect work directory failed: {error}"))?;
    if work_metadata.file_type().is_symlink() {
        return Err("--work-dir must not be a symbolic link".to_owned());
    }
    if arguments
        .work_dir
        .read_dir()
        .map_err(|error| format!("inspect work directory failed: {error}"))?
        .next()
        .is_some()
    {
        return Err("--work-dir must be empty".to_owned());
    }
    let (samples, checksum) = match arguments.case {
        Case::LoadEmpty
        | Case::LoadProse
        | Case::LoadMixed
        | Case::LoadNewline
        | Case::LoadLongLine
        | Case::LoadSourceLarge
        | Case::LoadLogLarge => run_load(arguments)?,
        Case::SearchEarly
        | Case::SearchMiddle
        | Case::SearchLate
        | Case::SearchAbsent
        | Case::SearchAdversarial => run_search(arguments)?,
        Case::SerializeProse => run_serialize(arguments)?,
        Case::SaveNewProse => run_save(arguments, false)?,
        Case::SaveReplaceProse => run_save(arguments, true)?,
    };
    emit_result(arguments.case, arguments.warmup, &samples, checksum)?;
    hold_if_requested(arguments.hold)
}

fn real_main() -> Result<(), String> {
    match parse_arguments()? {
        Action::Probe => probe_case_contract(),
        Action::List => emit_case_list(),
        Action::Run(arguments) => run(&arguments),
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("M1 benchmark worker failed: {error}");
            ExitCode::from(2)
        }
    }
}
