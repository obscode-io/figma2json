use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "figma2json")]
#[command(version, about = "Convert Figma .fig files to JSON")]
#[command(long_about = "Convert Figma .fig files to JSON\n\n\
    JSON output is pretty-printed by default with indentation.\n\n\
    A .fig saved from Figma (File > Save local copy) is a ZIP archive. Give an output directory:\n  \
    figma2json design.fig output-dir [--compact] [--keep-internal]\n  \
    -> output-dir/canvas.json, plus images/ and the other archive contents\n\n\
    A bare canvas.fig (already extracted) can go to a file or stdout:\n  \
    figma2json canvas.fig [-o output.json] [--compact] [-v]")]
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

    /// Original fig2json (pre-0.4) output: drop node ids/types, variable and style names,
    /// the root `variables`/`styles`, auto-layout alignment/sizing, constraints,
    /// scroll behavior, layout grids and override paths
    #[arg(long)]
    legacy: bool,

    /// Keep internal-only nodes (marked "internalOnly": true), including the hidden page
    /// with local copies of the library components, variables and styles the file uses
    #[arg(long)]
    keep_internal: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.verbose {
        eprintln!("Reading input file: {}", cli.input.display());
    }

    // Read input file
    let bytes = fs::read(&cli.input)
        .with_context(|| format!("Failed to read input file: {}", cli.input.display()))?;

    if cli.verbose {
        eprintln!("File size: {} bytes", bytes.len());
    }

    // Check if input is a ZIP container
    let is_zip = figma2json::parser::is_zip_container(&bytes);

    // Validate arguments based on file type
    if is_zip {
        // ZIP mode: require extract_dir, forbid -o
        let extract_dir = cli.extract_dir.as_ref()
            .ok_or_else(|| {
                anyhow!(
                    "{} is a ZIP archive (as saved by Figma). Give an output directory: figma2json {} <output-dir>",
                    cli.input.display(),
                    cli.input.display()
                )
            })?;

        if cli.output.is_some() {
            bail!("-o/--output can't be used with a ZIP archive: the JSON is written to <output-dir>/canvas.json");
        }

        // ZIP extraction mode
        handle_zip_mode(&bytes, extract_dir, cli.compact, cli.verbose, cli.raw, cli.legacy, cli.keep_internal)?;
    } else {
        // Regular .fig file mode
        if cli.verbose {
            eprintln!("Converting to JSON...");
        }

        // Determine base directory for image file operations
        let base_dir = if let Some(output_path) = &cli.output {
            output_path.parent()
        } else {
            // If outputting to stdout, use current directory
            Some(std::path::Path::new("."))
        };

        let options = figma2json::ConvertOptions {
            legacy: cli.legacy,
            keep_internal: cli.keep_internal,
        };
        let json = figma2json::convert_with_options(&bytes, base_dir, &options)
            .context("Failed to convert .fig file to JSON")?;

        if cli.verbose {
            eprintln!("Conversion successful!");
        }

        // Write output (pretty by default, compact if flag is set), streamed so the
        // whole document is never held twice in memory as a string
        match cli.output.as_ref() {
            Some(path) => {
                if cli.verbose {
                    eprintln!("Writing output to: {}", path.display());
                }
                write_json_file(path, &json, cli.compact)?;
                if cli.verbose {
                    eprintln!("Done!");
                }
            }
            None => {
                let stdout = std::io::stdout();
                let mut out = std::io::BufWriter::new(stdout.lock());
                let written = write_json(&mut out, &json, cli.compact)
                    .and_then(|_| writeln!(out).map_err(anyhow::Error::from))
                    .and_then(|_| out.flush().map_err(anyhow::Error::from));
                // The reader closing early (`| head`, `| jq` exiting) is not an error
                if let Err(e) = written {
                    if !is_broken_pipe(&e) {
                        return Err(e);
                    }
                }
            }
        }
        drop(json);

        // If --raw flag is set, also generate raw JSON file
        if cli.raw {
            if cli.verbose {
                eprintln!("Converting to raw JSON...");
            }

            let raw_json = figma2json::convert_raw(&bytes).context("Failed to convert .fig file to raw JSON")?;


            // Determine raw output path
            let raw_path = match cli.output.as_ref() {
                Some(path) => {
                    // Derive .raw.json from output path
                    let mut raw = path.clone();
                    raw.set_extension("raw.json");
                    raw
                }
                None => {
                    // Derive from input path
                    cli.input.with_extension("raw.json")
                }
            };

            if cli.verbose {
                eprintln!("Writing raw output to: {}", raw_path.display());
            }

            write_json_file(&raw_path, &raw_json, cli.compact)?;

            if cli.verbose {
                eprintln!("Raw JSON done!");
            }
        }
    }

    Ok(())
}

/// Handle ZIP extraction mode: extract all files and convert all .fig files found
fn handle_zip_mode(
    zip_bytes: &[u8],
    extract_dir: &PathBuf,
    compact: bool,
    verbose: bool,
    raw: bool,
    legacy: bool,
    keep_internal: bool,
) -> Result<()> {
    let options = figma2json::ConvertOptions { legacy, keep_internal };
    if verbose {
        eprintln!("ZIP file detected - extracting to: {}", extract_dir.display());
    }

    // Extract entire ZIP to directory
    figma2json::parser::extract_zip_to_directory(zip_bytes, extract_dir)
        .context("Failed to extract ZIP file")?;

    if verbose {
        eprintln!("ZIP extracted successfully");
        eprintln!("Searching for .fig files...");
    }

    // Find all .fig files in extracted contents
    let fig_files = find_fig_files(extract_dir)?;

    if fig_files.is_empty() {
        bail!("No .fig files found in ZIP archive");
    }

    let file_count = fig_files.len();

    if verbose {
        eprintln!("Found {} .fig file(s)", file_count);
    }

    // Convert each .fig file
    for fig_path in fig_files {
        let relative_path = fig_path.strip_prefix(extract_dir)
            .unwrap_or(&fig_path);

        if verbose {
            eprintln!("Converting: {}", relative_path.display());
        }

        // Read .fig file
        let fig_bytes = fs::read(&fig_path)
            .with_context(|| format!("Failed to read .fig file: {}", fig_path.display()))?;

        // Determine base directory for image file operations (parent of .fig file)
        let base_dir = fig_path.parent();

        // Convert to JSON
        let json = figma2json::convert_with_options(&fig_bytes, base_dir, &options)
            .with_context(|| format!("Failed to convert: {}", fig_path.display()))?;

        // Determine output path: same as .fig but with .json extension
        let output_path = fig_path.with_extension("json");

        // Write JSON file (pretty by default, compact if flag is set)
        write_json_file(&output_path, &json, compact)?;
        drop(json);

        if verbose {
            eprintln!("  → {}", output_path.strip_prefix(extract_dir).unwrap_or(&output_path).display());
        }

        // If --raw flag is set, also generate raw JSON file
        if raw {
            let raw_json = figma2json::convert_raw(&fig_bytes)
                .with_context(|| format!("Failed to convert to raw JSON: {}", fig_path.display()))?;


            // Determine raw output path: same as .fig but with .raw.json extension
            let raw_output_path = fig_path.with_extension("raw.json");

            // Write raw JSON file
            write_json_file(&raw_output_path, &raw_json, compact)?;

            if verbose {
                eprintln!("  → {}", raw_output_path.strip_prefix(extract_dir).unwrap_or(&raw_output_path).display());
            }
        }
    }

    if verbose {
        eprintln!("Done! Converted {} file(s)", file_count);
    }

    Ok(())
}

/// Recursively find all .fig files in a directory
fn find_fig_files(dir: &PathBuf) -> Result<Vec<PathBuf>> {
    let mut fig_files = Vec::new();

    fn visit_dir(dir: &PathBuf, fig_files: &mut Vec<PathBuf>) -> Result<()> {
        if dir.is_dir() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    visit_dir(&path, fig_files)?;
                } else if path.extension().and_then(|s| s.to_str()) == Some("fig") {
                    fig_files.push(path);
                }
            }
        }
        Ok(())
    }

    visit_dir(dir, &mut fig_files)?;
    Ok(fig_files)
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
fn write_json_file(path: &std::path::Path, json: &serde_json::Value, compact: bool) -> Result<()> {
    let file = fs::File::create(path)
        .with_context(|| format!("Failed to write output file: {}", path.display()))?;
    let mut out = std::io::BufWriter::new(file);
    write_json(&mut out, json, compact)
        .with_context(|| format!("Failed to write output file: {}", path.display()))?;
    out.flush()
        .with_context(|| format!("Failed to write output file: {}", path.display()))?;
    Ok(())
}

/// True when an error comes from writing to a closed pipe.
fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .map(|e| e.kind() == std::io::ErrorKind::BrokenPipe)
            .or_else(|| {
                cause
                    .downcast_ref::<serde_json::Error>()
                    .and_then(|e| e.io_error_kind())
                    .map(|kind| kind == std::io::ErrorKind::BrokenPipe)
            })
            .unwrap_or(false)
    })
}
