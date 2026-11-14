// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Witold Kaminski

use fsscanner::fsscanner_mt;
use pdf_to_text_wrapper::config::Config;
use pdf_to_text_wrapper::process_pdf_to_text::{process_pdf_to_text, PdfBackend};
use pdf_to_text_wrapper::process_text_to_token::{
    process_join_token, process_text_to_token, process_token_to_global,
};
use pdf_to_text_wrapper::error::{Error, Result};
use pdf_to_text_wrapper::service::start_service;
use std::path::{Path, PathBuf};
use token_db::TokenDb;

fn usage() {
    eprintln!("Usage:");
    eprintln!("  pdf_to_text [grobid|mineru] <pdf_input_dir|pdf_file> <output_dir>");
    eprintln!("  pdf_to_text start [grobid|mineru]");
}

/// Resolve the canonical output paths for a single input PDF.
fn single_pdf_outputs(input: &Path, output_root: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    if !input.is_file() || !input.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf")) {
        return Err(Error::InvalidPath("single-file input must be an existing PDF"));
    }
    let stem = input
        .file_stem()
        .ok_or(Error::InvalidPath("input PDF has no file stem"))?;
    let base = output_root.join(stem);
    Ok((base.with_extension("md"), base.with_extension("tok"), base.with_extension("tdb")))
}

/// Process one PDF without scanning other input files. The token database
/// merge and global remapping stages remain shared with directory mode.
fn process_single_pdf(
    config: &Config,
    backend: PdfBackend,
    input: &Path,
    output_root: &Path,
) -> Result<()> {
    let (text_output, token_output, db_output) = single_pdf_outputs(input, output_root)?;
    std::fs::create_dir_all(output_root)?;
    if fsscanner_mt::needs_update(input, &text_output).map_err(|_| Error::FsScanner)? {
        process_pdf_to_text(config, backend, input, &text_output)?;
    }
    if fsscanner_mt::needs_update(&text_output, &token_output).map_err(|_| Error::FsScanner)?
        || fsscanner_mt::needs_update(&text_output, &db_output).map_err(|_| Error::FsScanner)?
    {
        process_text_to_token(&text_output, &token_output, &db_output)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config = Config::load_or_create()?;

    if args.get(1).map(String::as_str) == Some("start") {
        let backend = match args.get(2) {
            Some(value) => value.parse::<PdfBackend>()?,
            None => config.backend,
        };

        if args.len() > 3 {
            usage();
            std::process::exit(1);
        }

        let mut child = start_service(&config, backend)?;
        let status = child.wait()?;
        if !status.success() {
            return Err(Error::ServiceFailed(status));
        }
        return Ok(());
    }

    if args.len() != 3 && args.len() != 4 {
        usage();
        std::process::exit(1);
    }

    let (backend, pdf_input, output_root) = if args.len() == 4 {
        (args[1].parse::<PdfBackend>()?, &args[2], &args[3])
    } else {
        (config.backend, &args[1], &args[2])
    };

    let single_file = Path::new(pdf_input).is_file();
    if !single_file && !Path::new(pdf_input).is_dir() {
        return Err(Error::InvalidPath("input must be an existing PDF or directory"));
    }

    //
    // Stage 1: PDF -> text
    //

    if single_file {
        process_single_pdf(&config, backend, Path::new(pdf_input), Path::new(output_root))?;
    } else {
        let workers = match backend {
            PdfBackend::Grobid => config.grobid.threads,
            PdfBackend::Mineru => config.mineru.threads,
        };

        let worker_config = config.clone();
        fsscanner_mt::process_dir_map_outdated_with_workers(
            pdf_input,
            output_root,
            "pdf",
            "md",
            workers,
            move |input, output| {
                process_pdf_to_text(&worker_config, backend, input, output)
                    .map_err(|_| fsscanner::Error::Callback("PDF conversion failed"))
            },
        )
        .map_err(|_| Error::FsScanner)?;

        //
        // Stage 2: text -> per-document token stream + lookup database
        //

        fsscanner_mt::process_dir_map_with_workers(
            pdf_input,
            output_root,
            "pdf",
            "tok",
            1,
            move |_pdf, token_output| {
                let text_input = token_output.with_extension("md");
                let db_output = token_output.with_extension("tdb");
                if fsscanner_mt::needs_update(&text_input, token_output)?
                    || fsscanner_mt::needs_update(&text_input, &db_output)?
                {
                    process_text_to_token(&text_input, token_output, &db_output)
                        .map_err(|_| fsscanner::Error::Callback("token conversion failed"))?;
                }
                Ok(())
            },
        )
        .map_err(|_| Error::FsScanner)?;
    }

    //
    // Stage 3: per-document lookup databases -> merged corpus database
    //

    let token_db_output = PathBuf::from(output_root).join("token_db.tdb");
    if fsscanner_mt::dir_needs_update(output_root, "tdb", &token_db_output)
        .map_err(|_| Error::FsScanner)?
    {
        println!("token merge -> {}", token_db_output.display());

        let token_db = fsscanner_mt::process_dir_state_and_map(
            TokenDb::new(),
            output_root,
            output_root,
            "tdb",
            "unused",
            {
                let token_db_output = token_db_output.clone();
                move |db, input, _| {
                    if input == token_db_output {
                        return Ok(());
                    }
                    process_join_token(db, input)
                        .map_err(|_| fsscanner::Error::Callback("token merge failed"))
                }
            },
        )
        .map_err(|_| Error::FsScanner)?;

        token_db.save(&token_db_output)?;
    }

    //
    // Stage 4: document-local token IDs -> corpus-global token IDs
    //

    let global_db = TokenDb::load(&token_db_output)?;
    fsscanner_mt::process_dir_map_with_workers(
        output_root,
        output_root,
        "tdb",
        "unused",
        1,
        {
            let token_db_output = token_db_output.clone();
            move |local_db_input, _| {
                if local_db_input == token_db_output {
                    return Ok(());
                }

                let token_input = local_db_input.with_extension("tok");
                let stem = local_db_input
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .ok_or(fsscanner::Error::Callback("invalid token database path"))?;
                let global_token_output = local_db_input.with_file_name(format!("{stem}_glob.tok"));

                if fsscanner_mt::needs_update(&token_db_output, &global_token_output)? {
                    process_token_to_global(
                        &token_input,
                        local_db_input,
                        &global_db,
                        &global_token_output,
                    )
                    .map_err(|_| fsscanner::Error::Callback("global token remap failed"))?;
                }
                Ok(())
            }
        },
    )
    .map_err(|_| Error::FsScanner)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::single_pdf_outputs;
    use std::path::Path;

    #[test]
    fn single_pdf_maps_to_normal_output_layout() {
        let dir = std::env::temp_dir();
        let name = format!("pdf_to_text_single_input_{}_{}.pdf", std::process::id(), line!());
        let input = dir.join(name);
        std::fs::write(&input, b"%PDF-1.4").unwrap();
        let out = dir.join("test_output");
        let (md, tok, tdb) = single_pdf_outputs(&input, &out).unwrap();
        assert_eq!(md.file_name().unwrap(), input.with_extension("md").file_name().unwrap());
        assert_eq!(tok.file_name().unwrap(), input.with_extension("tok").file_name().unwrap());
        assert_eq!(tdb.file_name().unwrap(), input.with_extension("tdb").file_name().unwrap());
        std::fs::remove_file(input).unwrap();
    }

    #[test]
    fn single_pdf_rejects_non_pdf() {
        assert!(single_pdf_outputs(Path::new("/not-a-pdf.txt"), Path::new("/tmp")).is_err());
    }
}
