//! The command-line interface shared by the `figma2json` and `f2j` binaries.
//!
//! Available with the `cli` feature (on by default). Both binaries call [`run`]
//! with their own name, which is used in `--help`, `--version` and error messages.

use crate::ConvertOptions;
use anyhow::{anyhow, bail, Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser};
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(version, about = "Convert Figma .fig files to JSON")]
struct Cli {
    /// Input .fig or .zip file path
    input: PathBuf,

    /// Directory to extract ZIP contents (required for ZIP files, converts all .fig files found)
    extract_dir: Option<PathBuf>,

    /// Output JSON file path (default: stdout) - Cannot be used with extract_dir
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Compact JSON output (default is pretty-printed with indentation)
    #[arg(long)]
    compact: bool,

    /// Verbose output for debugging
    #[arg(short, long)]
    verbose: bool,

    /// Generate both transformed .json and raw .raw.json files (without transformations)
    #[arg(long)]
    raw: bool,

    /// Smaller output without design context: drop node ids/types, variable and style names,
    /// the root `variables`/`styles`, auto-layout alignment/sizing, constraints,
    /// scroll behavior, layout grids and override paths
    #[arg(long)]
    legacy: bool,

    /// Keep internal-only nodes (marked "internalOnly": true), including the hidden page
    /// with local copies of the library components, variables and styles the file uses
    #[arg(long)]
    keep_internal: bool,
}

impl Cli {
    fn convert_options(&self) -> ConvertOptions {
        ConvertOptions {
            legacy: self.legacy,
            keep_internal: self.keep_internal,
        }
    }
}

/// Where the transformed JSON of one conversion is written.
#[derive(Clone, Copy)]
enum Output<'a> {
    Stdout,
    File(&'a Path),
}

/// What is being converted, which decides how progress and errors are reported.
#[derive(Clone, Copy)]
enum Source<'a> {
    /// The input file given on the command line (a bare canvas.fig).
    Input,
    /// A .fig found in the directory a ZIP archive was extracted to.
    ArchiveEntry { fig_path: &'a Path, root: &'a Path },
}

impl Source<'_> {
    fn conversion_failed(self, raw: bool) -> String {
        match (self, raw) {
            (Source::Input, false) => "Failed to convert .fig file to JSON".to_owned(),
            (Source::Input, true) => "Failed to convert .fig file to raw JSON".to_owned(),
            (Source::ArchiveEntry { fig_path, .. }, false) => {
                format!("Failed to convert: {}", fig_path.display())
            }
            (Source::ArchiveEntry { fig_path, .. }, true) => {
                format!("Failed to convert to raw JSON: {}", fig_path.display())
            }
        }
    }

    /// Verbose note printed before converting (`raw`: the untransformed output).
    fn log_converting(self, raw: bool) {
        match (self, raw) {
            (Source::Input, false) => eprintln!("Converting to JSON..."),
            (Source::Input, true) => eprintln!("Converting to raw JSON..."),
            (Source::ArchiveEntry { fig_path, root }, false) => {
                eprintln!("Converting: {}", relative_to(fig_path, root).display());
            }
            (Source::ArchiveEntry { .. }, true) => {}
        }
    }

    /// Verbose note printed once the JSON is in memory, before it is written.
    fn log_writing(self, output: Output<'_>, raw: bool) {
        match (self, output) {
            (Source::Input, Output::File(path)) if raw => {
                eprintln!("Writing raw output to: {}", path.display());
            }
            (Source::Input, Output::File(path)) => {
                eprintln!("Conversion successful!");
                eprintln!("Writing output to: {}", path.display());
            }
            (Source::Input, Output::Stdout) => eprintln!("Conversion successful!"),
            (Source::ArchiveEntry { .. }, _) => {}
        }
    }

    /// Verbose note printed after the JSON was written.
    fn log_written(self, output: Output<'_>, raw: bool) {
        match (self, output) {
            (Source::Input, Output::File(_)) if raw => eprintln!("Raw JSON done!"),
            (Source::Input, Output::File(_)) => eprintln!("Done!"),
            (Source::ArchiveEntry { root, .. }, Output::File(path)) => {
                eprintln!("  → {}", relative_to(path, root).display());
            }
            (_, Output::Stdout) => {}
        }
    }
}

fn long_about(name: &str) -> String {
    format!(
        "Convert Figma .fig files to JSON\n\n\
         JSON output is pretty-printed by default with indentation.\n\n\
         A .fig saved from Figma (File > Save local copy) is a ZIP archive. Give an output directory:\n  \
         {name} design.fig output-dir [--compact] [--keep-internal]\n  \
         -> output-dir/canvas.json, plus images/ and the other archive contents\n\n\
         A bare canvas.fig (already extracted) can go to a file or stdout:\n  \
         {name} canvas.fig [-o output.json] [--compact] [-v]\n\n\
         figma2json and f2j are the same program."
    )
}

/// Parse the command line and run the conversion.
///
/// `name` is the program name shown in help, version and error messages
/// (`"figma2json"` or `"f2j"`).
///
/// # Errors
/// Returns an error when the input cannot be read, converted or written. A
/// closed stdout (for example `f2j canvas.fig | head`) is not an error.
pub fn run(name: &'static str) -> Result<()> {
    let matches = Cli::command()
        .name(name)
        .bin_name(name)
        .long_about(long_about(name))
        .get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());

    if cli.verbose {
        eprintln!("Reading input file: {}", cli.input.display());
    }

    let bytes = fs::read(&cli.input)
        .with_context(|| format!("Failed to read input file: {}", cli.input.display()))?;

    if cli.verbose {
        eprintln!("File size: {} bytes", bytes.len());
    }

    if crate::parser::is_zip_container(&bytes) {
        let extract_dir = cli.extract_dir.as_deref().ok_or_else(|| {
            anyhow!(
                "{} is a ZIP archive (as saved by Figma). Give an output directory: {name} {} <output-dir>",
                cli.input.display(),
                cli.input.display()
            )
        })?;

        if cli.output.is_some() {
            bail!("-o/--output can't be used with a ZIP archive: the JSON is written to <output-dir>/canvas.json");
        }

        convert_zip(&cli, &bytes, extract_dir)
    } else {
        convert_input(&cli, &bytes)
    }
}

/// Convert a bare .fig given on the command line, to `-o` or stdout.
fn convert_input(cli: &Cli, bytes: &[u8]) -> Result<()> {
    let (output, base_dir) = match cli.output.as_deref() {
        Some(path) => (Output::File(path), path.parent()),
        None => (Output::Stdout, Some(Path::new("."))),
    };
    let raw_output = cli
        .output
        .as_deref()
        .unwrap_or(&cli.input)
        .with_extension("raw.json");

    convert_one(cli, Source::Input, bytes, base_dir, output, &raw_output)
}

/// Extract a ZIP archive to `extract_dir` and convert every .fig file it contains
/// to a .json (and with `--raw`, a .raw.json) next to it.
fn convert_zip(cli: &Cli, zip_bytes: &[u8], extract_dir: &Path) -> Result<()> {
    if cli.verbose {
        eprintln!(
            "ZIP file detected - extracting to: {}",
            extract_dir.display()
        );
    }

    crate::parser::extract_zip_to_directory(zip_bytes, extract_dir)
        .context("Failed to extract ZIP file")?;

    if cli.verbose {
        eprintln!("ZIP extracted successfully");
        eprintln!("Searching for .fig files...");
    }

    let fig_files = find_fig_files(extract_dir)?;
    if fig_files.is_empty() {
        bail!("No .fig files found in ZIP archive");
    }

    if cli.verbose {
        eprintln!("Found {} .fig file(s)", fig_files.len());
    }

    for fig_path in &fig_files {
        let fig_bytes = fs::read(fig_path)
            .with_context(|| format!("Failed to read .fig file: {}", fig_path.display()))?;
        let source = Source::ArchiveEntry {
            fig_path,
            root: extract_dir,
        };
        let output = fig_path.with_extension("json");
        let raw_output = fig_path.with_extension("raw.json");

        convert_one(
            cli,
            source,
            &fig_bytes,
            fig_path.parent(),
            Output::File(&output),
            &raw_output,
        )?;
    }

    if cli.verbose {
        eprintln!("Done! Converted {} file(s)", fig_files.len());
    }

    Ok(())
}

/// Convert one .fig file and write its JSON to `output`; with `--raw`, also write
/// the untransformed JSON to `raw_output`.
///
/// The transformed JSON is freed before the raw one is built, so the two are
/// never in memory at the same time.
fn convert_one(
    cli: &Cli,
    source: Source<'_>,
    bytes: &[u8],
    base_dir: Option<&Path>,
    output: Output<'_>,
    raw_output: &Path,
) -> Result<()> {
    if cli.verbose {
        source.log_converting(false);
    }
    let json = crate::convert_with_options(bytes, base_dir, &cli.convert_options())
        .with_context(|| source.conversion_failed(false))?;
    write_output(cli, source, output, &json, false)?;
    drop(json);

    if cli.raw {
        if cli.verbose {
            source.log_converting(true);
        }
        let raw_json = crate::convert_raw(bytes).with_context(|| source.conversion_failed(true))?;
        write_output(cli, source, Output::File(raw_output), &raw_json, true)?;
    }

    Ok(())
}

fn write_output(
    cli: &Cli,
    source: Source<'_>,
    output: Output<'_>,
    json: &serde_json::Value,
    raw: bool,
) -> Result<()> {
    if cli.verbose {
        source.log_writing(output, raw);
    }
    match output {
        Output::File(path) => write_json_file(path, json, cli.compact)?,
        Output::Stdout => write_json_stdout(json, cli.compact)?,
    }
    if cli.verbose {
        source.log_written(output, raw);
    }
    Ok(())
}

/// Recursively find all .fig files under `dir`, sorted so they are converted in a
/// stable order.
fn find_fig_files(dir: &Path) -> Result<Vec<PathBuf>> {
    fn visit(dir: &Path, fig_files: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(&path, fig_files)?;
            } else if path.extension().is_some_and(|ext| ext == "fig") {
                fig_files.push(path);
            }
        }
        Ok(())
    }

    let mut fig_files = Vec::new();
    if dir.is_dir() {
        visit(dir, &mut fig_files)?;
    }
    fig_files.sort();
    Ok(fig_files)
}

/// `path` relative to `root`, or `path` itself when it is not under `root`.
fn relative_to<'a>(path: &'a Path, root: &Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

/// Serialize JSON to a writer, pretty by default or compact.
fn write_json<W: Write>(out: &mut W, json: &serde_json::Value, compact: bool) -> Result<()> {
    if compact {
        serde_json::to_writer(&mut *out, json)?;
    } else {
        serde_json::to_writer_pretty(&mut *out, json)?;
    }
    Ok(())
}

/// Stream JSON to a file through a buffer instead of building one big string first.
fn write_json_file(path: &Path, json: &serde_json::Value, compact: bool) -> Result<()> {
    let context = || format!("Failed to write output file: {}", path.display());
    let file = fs::File::create(path).with_context(context)?;
    let mut out = BufWriter::new(file);
    write_json(&mut out, json, compact).with_context(context)?;
    out.flush().with_context(context)?;
    Ok(())
}

/// Stream JSON to stdout followed by a newline. The reader closing early
/// (`| head`, `| jq` exiting) is not an error.
fn write_json_stdout(json: &serde_json::Value, compact: bool) -> Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    let written = write_json(&mut out, json, compact)
        .and_then(|()| writeln!(out).map_err(anyhow::Error::from))
        .and_then(|()| out.flush().map_err(anyhow::Error::from));
    match written {
        Err(e) if !is_broken_pipe(&e) => Err(e),
        _ => Ok(()),
    }
}

/// True when an error comes from writing to a closed pipe.
fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let kind = cause
            .downcast_ref::<io::Error>()
            .map(io::Error::kind)
            .or_else(|| {
                cause
                    .downcast_ref::<serde_json::Error>()
                    .and_then(serde_json::Error::io_error_kind)
            });
        kind == Some(io::ErrorKind::BrokenPipe)
    })
}
