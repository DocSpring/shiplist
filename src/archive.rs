//! Lists the files in a package or other archive: tar, gzipped tar and zip files, and the packages built on them
//! (npm and Python source packages, `.crate`, Helm charts, wheels, `.nupkg`, `.jar`, `.gem`, Hex packages).

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result, bail};

const GZIP: [u8; 2] = [0x1f, 0x8b];
const ZIP_LOCAL: [u8; 4] = *b"PK\x03\x04";
const ZIP_EMPTY: [u8; 4] = *b"PK\x05\x06";

fn normalize(path: &str) -> String {
    path.trim_start_matches("./").trim_start_matches('/').to_string()
}

/// Every file and symlink in the archive (not directories), as paths without a leading `/`. A `.gem` or Hex package
/// lists the files of the tarball inside it that holds the package's contents.
pub fn archive_files(path: &Path) -> Result<BTreeSet<String>> {
    let mut file = File::open(path).with_context(|| format!("couldn't open {}", path.display()))?;
    let mut magic = [0u8; 4];
    let n = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    let magic = &magic[..n];
    let files = if magic.starts_with(&ZIP_LOCAL) || magic.starts_with(&ZIP_EMPTY) {
        zip_files(&mut file)
    } else if magic.starts_with(&GZIP) {
        tar_files(flate2::read::GzDecoder::new(BufReader::new(file)))
    } else if is_tar(path)? {
        tar_files(BufReader::new(file))
    } else {
        bail!("{} isn't a tar, gzipped tar or zip file", path.display())
    };
    files.with_context(|| format!("couldn't read {}", path.display()))
}

/// Whether the file has a tar header: "ustar" at byte 257.
fn is_tar(path: &Path) -> Result<bool> {
    let mut header = [0u8; 262];
    let mut file = File::open(path)?;
    Ok(file.read(&mut header)? == header.len() && &header[257..262] == b"ustar")
}

/// The contents tarball inside a package that wraps one: `data.tar.gz` in a `.gem`, `contents.tar.gz` in a Hex
/// package. Each is next to a metadata file, so a tarball that merely contains a `data.tar.gz` isn't mistaken for one.
fn inner_tarball(members: &BTreeSet<String>) -> Option<&'static str> {
    if members.contains("metadata.gz") && members.contains("data.tar.gz") {
        Some("data.tar.gz")
    } else if members.contains("metadata.config") && members.contains("contents.tar.gz") {
        Some("contents.tar.gz")
    } else {
        None
    }
}

fn list_tar(reader: impl Read, mut keep: impl FnMut(&str, &mut dyn Read) -> Result<()>) -> Result<BTreeSet<String>> {
    let mut archive = tar::Archive::new(reader);
    let mut files = BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let path = normalize(entry.path()?.to_str().context("the archive has a path that isn't UTF-8")?);
        if path.is_empty() {
            continue;
        }
        keep(&path, &mut entry)?;
        files.insert(path);
    }
    Ok(files)
}

fn tar_files(reader: impl Read) -> Result<BTreeSet<String>> {
    // Keep the two possible inner tarballs while listing, so the outer archive is read once.
    let mut inner: Vec<(String, Vec<u8>)> = Vec::new();
    let members = list_tar(reader, |path, entry| {
        if path == "data.tar.gz" || path == "contents.tar.gz" {
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            inner.push((path.to_string(), data));
        }
        Ok(())
    })?;
    let Some(name) = inner_tarball(&members) else {
        return Ok(members);
    };
    let data = inner.into_iter().find(|(p, _)| p == name).map(|(_, d)| d).expect("kept while listing");
    list_tar(flate2::read::GzDecoder::new(data.as_slice()), |_, _| Ok(()))
        .with_context(|| format!("couldn't read {name}"))
}

fn u16_at(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}

fn u32_at(b: &[u8], at: usize) -> u64 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes")) as u64
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

/// Reads the names in a zip file's central directory, which lists every entry, so no entry is decompressed.
fn zip_files(file: &mut File) -> Result<BTreeSet<String>> {
    let len = file.seek(SeekFrom::End(0))?;
    // The end of central directory record is 22 bytes, followed by a comment of up to 65,535 bytes.
    let tail_len = len.min(22 + 65_535);
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)?;
    let eocd = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| tail[i..i + 4] == *b"PK\x05\x06" && i + 22 + u16_at(&tail, i + 20) == tail.len())
        .context("no end of central directory record: the zip file is cut short or isn't a zip file")?;
    let mut entries = u16_at(&tail, eocd + 10) as u64;
    let mut cd_size = u32_at(&tail, eocd + 12);
    let mut cd_offset = u32_at(&tail, eocd + 16);

    if entries == 0xffff || cd_size == 0xffff_ffff || cd_offset == 0xffff_ffff {
        // Zip64: a locator just before the record points at the zip64 end of central directory record.
        let locator = eocd.checked_sub(20).context("a zip64 file without a zip64 locator")?;
        if tail[locator..locator + 4] != *b"PK\x06\x07" {
            bail!("a zip64 file without a zip64 locator");
        }
        file.seek(SeekFrom::Start(u64_at(&tail, locator + 8)))?;
        let mut record = [0u8; 56];
        file.read_exact(&mut record)?;
        if record[..4] != *b"PK\x06\x06" {
            bail!("the zip64 locator doesn't point at a zip64 end of central directory record");
        }
        entries = u64_at(&record, 32);
        cd_size = u64_at(&record, 40);
        cd_offset = u64_at(&record, 48);
    }
    if cd_offset.checked_add(cd_size).is_none_or(|end| end > len) {
        bail!("the central directory runs past the end of the file");
    }

    file.seek(SeekFrom::Start(cd_offset))?;
    let mut cd = vec![0u8; cd_size as usize];
    file.read_exact(&mut cd)?;
    let mut files = BTreeSet::new();
    let mut at = 0;
    for _ in 0..entries {
        if at + 46 > cd.len() || cd[at..at + 4] != *b"PK\x01\x02" {
            bail!("the central directory is shorter than the {entries} entries it claims");
        }
        let (name_len, extra_len, comment_len) = (u16_at(&cd, at + 28), u16_at(&cd, at + 30), u16_at(&cd, at + 32));
        let name = cd.get(at + 46..at + 46 + name_len).context("an entry's name runs past the central directory")?;
        let name = std::str::from_utf8(name).context("the zip file has a name that isn't UTF-8")?;
        if !name.ends_with('/') {
            let path = normalize(name);
            if !path.is_empty() {
                files.insert(path);
            }
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(files)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (path, data) in entries {
            let mut h = tar::Header::new_ustar();
            if path.ends_with('/') {
                h.set_entry_type(tar::EntryType::Directory);
            }
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    pub fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// A zip file with stored (uncompressed) entries and the given comment, written by hand.
    pub fn zip_bytes(names: &[&str], comment: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for name in names {
            let offset = out.len() as u32;
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            cd.extend_from_slice(b"PK\x01\x02");
            cd.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
            cd.extend_from_slice(&[0; 12]);
            cd.extend_from_slice(&offset.to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment.as_bytes());
        out
    }

    fn list(bytes: &[u8]) -> Vec<String> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bytes).unwrap();
        archive_files(tmp.path()).unwrap().into_iter().collect()
    }

    #[test]
    fn reads_tar_and_gzipped_tar() {
        let tar = tar_bytes(&[("package/", b""), ("package/package.json", b"{}"), ("./package/lib/a.js", b"x")]);
        assert_eq!(list(&tar), ["package/lib/a.js", "package/package.json"]);
        assert_eq!(list(&gzip(&tar)), ["package/lib/a.js", "package/package.json"]);
    }

    #[test]
    fn reads_the_contents_of_a_gem_and_a_hex_package() {
        let data = gzip(&tar_bytes(&[("lib/a.rb", b"x"), ("spec/a_spec.rb", b"x")]));
        let gem = tar_bytes(&[("metadata.gz", b"x"), ("data.tar.gz", &data), ("checksums.yaml.gz", b"x")]);
        assert_eq!(list(&gem), ["lib/a.rb", "spec/a_spec.rb"]);
        let hex = tar_bytes(&[("VERSION", b"3"), ("metadata.config", b"x"), ("contents.tar.gz", &data)]);
        assert_eq!(list(&hex), ["lib/a.rb", "spec/a_spec.rb"]);
        // A tarball that only happens to hold a data.tar.gz is listed as it is
        assert_eq!(list(&tar_bytes(&[("data.tar.gz", &data)])), ["data.tar.gz"]);
    }

    #[test]
    fn reads_zip_central_directory() {
        let names = ["pkg/", "pkg/__init__.py", "pkg-1.0.dist-info/RECORD"];
        assert_eq!(list(&zip_bytes(&names, "")), ["pkg-1.0.dist-info/RECORD", "pkg/__init__.py"]);
        // A comment that contains the end-of-directory signature doesn't confuse the search
        assert_eq!(list(&zip_bytes(&["a.txt"], "PK\x05\x06 not a record")), ["a.txt"]);
        assert!(list(&zip_bytes(&[], "")).is_empty());
    }

    #[test]
    fn rejects_other_files_and_cut_short_zips() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"just some text, long enough to not be anything").unwrap();
        assert!(archive_files(tmp.path()).unwrap_err().to_string().contains("isn't a tar, gzipped tar or zip"));
        let zip = zip_bytes(&["a.txt", "b.txt"], "");
        std::fs::write(tmp.path(), &zip[..zip.len() - 5]).unwrap();
        assert!(archive_files(tmp.path()).is_err());
    }
}
