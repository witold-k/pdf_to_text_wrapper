// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Witold Kaminski

//! Convert configured sources to canonical UTF-8 text.

use crate::config::Config;
use crate::error::{Error, Result};
use crate::process_pdf_to_text::{process_pdf_to_text, PdfBackend};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Preserve the existing PDF layout while avoiding collisions between files
/// such as report.pdf and report.docx within one input directory.
pub fn mapped_output(input: &Path, mapped: &Path, suffix: &str) -> Result<PathBuf> {
    if is_pdf(input) {
        return Ok(mapped.to_path_buf());
    }
    let name = input
        .file_name()
        .ok_or(Error::InvalidPath("source path has no file name"))?
        .to_string_lossy();
    Ok(mapped.with_file_name(format!("{name}.{suffix}")))
}

fn is_pdf(input: &Path) -> bool {
    input.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

/// PDF uses the configured extractor; DOCX uses OOXML; any other configured
/// source suffix is interpreted as a plain UTF-8 text document.
pub fn process_source_to_text(
    config: &Config,
    backend: PdfBackend,
    input: &Path,
    output: &Path,
) -> Result<()> {
    if is_pdf(input) {
        return process_pdf_to_text(config, backend, input, output);
    }
    let text = if input.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("docx")) {
        docx_text(input)?
    } else {
        fs::read_to_string(input)?
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, text)?;
    Ok(())
}

fn docx_text(input: &Path) -> Result<String> {
    // DOCX is a ZIP archive containing OOXML. We use the system unzip utility
    // rather than implementing another archive parser in this orchestrator.
    let result = Command::new("unzip")
        .arg("-p")
        .arg(input)
        .arg("word/document.xml")
        .output()?;
    if !result.status.success() {
        return Err(Error::InvalidPath("could not extract word/document.xml from DOCX"));
    }
    let xml = String::from_utf8(result.stdout)
        .map_err(|_| Error::InvalidPath("DOCX document.xml is not UTF-8"))?;
    let document = roxmltree::Document::parse(&xml)?;
    let ns = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let mut text = String::new();
    for paragraph in document.descendants().filter(|node| node.has_tag_name((ns, "p"))) {
        for node in paragraph.descendants().filter(|node| node.is_element()) {
            if node.has_tag_name((ns, "t")) {
                if let Some(value) = node.text() {
                    text.push_str(value);
                }
            } else if node.has_tag_name((ns, "tab")) {
                text.push('\t');
            } else if node.has_tag_name((ns, "br")) || node.has_tag_name((ns, "cr")) {
                text.push('\n');
            }
        }
        text.push('\n');
    }
    Ok(text)
}


/// Resolve canonical output paths for one configured input file.
pub fn single_file_outputs(
    config: &Config,
    input: &Path,
    output_root: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf)> {
    if !input.is_file() || !config.accepts(input) {
        return Err(Error::InvalidPath("single-file input must match scanendfilter"));
    }
    let stem = input
        .file_stem()
        .ok_or(Error::InvalidPath("input source has no file stem"))?;
    let text = mapped_output(input, &output_root.join(stem).with_extension("md"), "md")?;
    Ok((text.clone(), text.with_extension("tok"), text.with_extension("tdb")))
}
