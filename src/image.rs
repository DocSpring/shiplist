//! Reads the final filesystem of an image saved with `docker save`, without Docker.

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Deserialize)]
struct ManifestEntry {
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

fn normalize(path: &str) -> String {
    path.trim_start_matches("./").trim_start_matches('/').trim_end_matches('/').to_string()
}

fn remove_tree(files: &mut BTreeSet<String>, path: &str) {
    files.remove(path);
    let prefix = format!("{path}/");
    let doomed: Vec<String> = files.range(prefix.clone()..).take_while(|f| f.starts_with(&prefix)).cloned().collect();
    for f in doomed {
        files.remove(&f);
    }
}

/// Applies one layer to the set of files: its whiteouts first (they hide files from lower layers), then its entries.
fn apply_layer(files: &mut BTreeSet<String>, layer: impl Read) -> Result<()> {
    let mut archive = tar::Archive::new(layer);
    let mut added = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let path = normalize(entry.path()?.to_str().context("a layer has a path that isn't UTF-8")?);
        let (parent, name) = match path.rsplit_once('/') {
            Some((p, n)) => (Some(p), n),
            None => (None, path.as_str()),
        };
        if name == ".wh..wh..opq" {
            match parent {
                Some(p) => {
                    let prefix = format!("{p}/");
                    let doomed: Vec<String> =
                        files.range(prefix.clone()..).take_while(|f| f.starts_with(&prefix)).cloned().collect();
                    for f in doomed {
                        files.remove(&f);
                    }
                }
                None => files.clear(),
            }
        } else if let Some(hidden) = name.strip_prefix(".wh.") {
            let target = match parent {
                Some(p) => format!("{p}/{hidden}"),
                None => hidden.to_string(),
            };
            remove_tree(files, &target);
        } else if !entry.header().entry_type().is_dir() && !path.is_empty() {
            added.push(path);
        }
    }
    files.extend(added);
    Ok(())
}

/// Every file and symlink in the image's final filesystem, as paths without a leading `/`.
pub fn image_files(tarball: &Path) -> Result<BTreeSet<String>> {
    let open = || File::open(tarball).with_context(|| format!("couldn't open {}", tarball.display()));

    // One pass over the outer tar: where each member's data starts, and the manifest.
    let mut offsets: HashMap<String, (u64, u64)> = HashMap::new();
    let mut manifest = None;
    let mut outer = tar::Archive::new(BufReader::new(open()?));
    for entry in outer.entries()? {
        let mut entry = entry?;
        let path = normalize(entry.path()?.to_str().context("the image tarball has a path that isn't UTF-8")?);
        if path == "manifest.json" {
            let mut text = String::new();
            entry.read_to_string(&mut text)?;
            manifest = Some(text);
        } else {
            offsets.insert(path, (entry.raw_file_position(), entry.size()));
        }
    }
    let Some(manifest) = manifest else {
        bail!("{} has no manifest.json: pass a tarball made by `docker save <image> -o image.tar`", tarball.display());
    };
    let manifest: Vec<ManifestEntry> = serde_json::from_str(&manifest).context("couldn't parse manifest.json")?;
    let [image] = manifest.as_slice() else {
        bail!("{} holds {} images: save exactly one", tarball.display(), manifest.len());
    };

    let mut files = BTreeSet::new();
    let mut file = open()?;
    for layer in &image.layers {
        let &(offset, size) = offsets
            .get(&normalize(layer))
            .with_context(|| format!("the manifest names layer {layer}, which isn't in the tarball"))?;
        file.seek(SeekFrom::Start(offset))?;
        let mut magic = [0u8; 2];
        let mut reader = BufReader::new((&mut file).take(size));
        let n = reader.read(&mut magic)?;
        let head = std::io::Cursor::new(magic[..n].to_vec()).chain(reader);
        if n == 2 && magic == [0x1f, 0x8b] {
            apply_layer(&mut files, flate2::read::GzDecoder::new(head))
        } else if n == 2 && magic == [0x28, 0xb5] {
            bail!("layer {layer} is zstd-compressed, which shiplist can't read yet: save the image with `docker save`")
        } else {
            apply_layer(&mut files, head)
        }
        .with_context(|| format!("couldn't read layer {layer}"))?;
    }
    Ok(files)
}

/// The files under `root` (such as `/app`), relative to it.
pub fn under_root(files: &BTreeSet<String>, root: &str) -> Vec<String> {
    let root = normalize(root);
    if root.is_empty() {
        return files.iter().cloned().collect();
    }
    let prefix = format!("{root}/");
    files.iter().filter_map(|f| f.strip_prefix(&prefix)).map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(entries: &[&str]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for path in entries {
            let mut h = tar::Header::new_gnu();
            if path.ends_with('/') {
                h.set_entry_type(tar::EntryType::Directory);
                h.set_size(0);
            } else {
                h.set_size(1);
            }
            h.set_mode(0o644);
            h.set_cksum();
            let data: &[u8] = if path.ends_with('/') { b"" } else { b"x" };
            b.append_data(&mut h, path, data).unwrap();
        }
        b.into_inner().unwrap()
    }

    pub fn save(dir: &Path, layers: &[Vec<u8>], gzip_second: bool) -> std::path::PathBuf {
        let mut b = tar::Builder::new(Vec::new());
        let mut names = Vec::new();
        for (i, data) in layers.iter().enumerate() {
            let name = format!("blobs/sha256/layer{i}");
            let data = if gzip_second && i == 1 {
                let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                std::io::Write::write_all(&mut e, data).unwrap();
                e.finish().unwrap()
            } else {
                data.clone()
            };
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, &name, data.as_slice()).unwrap();
            names.push(format!("\"{name}\""));
        }
        let manifest =
            format!("[{{\"Config\":\"blobs/sha256/cfg\",\"RepoTags\":[\"x:1\"],\"Layers\":[{}]}}]", names.join(","));
        let mut h = tar::Header::new_gnu();
        h.set_size(manifest.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, "manifest.json", manifest.as_bytes()).unwrap();
        let path = dir.join("image.tar");
        std::fs::write(&path, b.into_inner().unwrap()).unwrap();
        path
    }

    #[test]
    fn applies_layers_and_whiteouts() {
        let tmp = tempfile::tempdir().unwrap();
        let base = layer(&["etc/", "etc/os-release", "app/", "app/a.rb", "app/docs/x.md", "app/old/1", "app/cache/1"]);
        let top = layer(&["app/.wh.a.rb", "app/.wh.docs", "app/old/.wh..wh..opq", "app/old/2", "app/b.rb"]);
        let tarball = save(tmp.path(), &[base, top], true);
        let files = image_files(&tarball).unwrap();
        assert_eq!(under_root(&files, "/app"), ["b.rb", "cache/1", "old/2"]);
        assert!(files.contains("etc/os-release"));
    }
}
