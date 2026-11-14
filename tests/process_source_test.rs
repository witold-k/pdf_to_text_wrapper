// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Witold Kaminski

use pdf_to_text_wrapper::config::Config;
use pdf_to_text_wrapper::process_source::{mapped_output, single_file_outputs};
use std::path::Path;

#[test]
fn pdf_layout_is_unchanged() {
    assert_eq!(
        mapped_output(Path::new("/in/report.pdf"), Path::new("/out/report.md"), "md").unwrap(),
        Path::new("/out/report.md")
    );
}

#[test]
fn different_suffixes_have_distinct_output_names() {
    assert_eq!(
        mapped_output(Path::new("/in/report.docx"), Path::new("/out/report.md"), "md").unwrap(),
        Path::new("/out/report.docx.md")
    );
    assert_eq!(
        mapped_output(Path::new("/in/report.txt"), Path::new("/out/report.tok"), "tok").unwrap(),
        Path::new("/out/report.txt.tok")
    );
}

#[test]
fn configured_single_files_map_to_output_paths() {
    let config = Config::default();
    let dir = std::env::temp_dir();
    for extension in ["pdf", "docx"] {
        let input = dir.join(format!(
            "pdf_to_text_single_input_{}_{}.{}",
            std::process::id(), line!(), extension
        ));
        std::fs::write(&input, b"dummy").unwrap();
        let out = dir.join("test_output");
        let (md, tok, tdb) = single_file_outputs(&config, &input, &out).unwrap();
        let expected_md = mapped_output(&input, &out.join(input.file_stem().unwrap()).with_extension("md"), "md").unwrap();
        assert_eq!(md, expected_md);
        assert_eq!(tok, md.with_extension("tok"));
        assert_eq!(tdb, md.with_extension("tdb"));
        std::fs::remove_file(input).unwrap();
    }
}

#[test]
fn configurable_suffixes_allow_additional_plain_text_formats() {
    let mut config = Config::default();
    config.scanendfilter.push(".txt".to_string());
    assert!(config.accepts(Path::new("/docs/document.txt")));
    assert!(config.accepts(Path::new("/docs/document.PDF")));
    assert_eq!(config.scan_extensions().unwrap(), vec!["pdf", "docx", "txt"]);
}

#[test]
fn single_file_rejects_unconfigured_suffix() {
    let config = Config::default();
    assert!(!config.accepts(Path::new("/docs/data.xml")));
    assert!(single_file_outputs(&config, Path::new("/not-a-doc.xml"), Path::new("/tmp")).is_err());
}

#[test]
fn missing_suffixes_are_rejected() {
    let mut value = serde_json::to_value(Config::default()).unwrap();
    value.as_object_mut().unwrap().remove("scanendfilter");
    let error = serde_json::from_value::<Config>(value).unwrap_err();
    assert!(error.to_string().contains("missing field `scanendfilter`"));
}
