use crate::archive::{GLOBAL_HEADER, GNU_NAME_TABLE_ID};
use crate::header::Header;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{Error, ErrorKind, Result};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

/// A structure for building Common or BSD-variant archives (the archive format
/// typically used on e.g. BSD and Mac OS X systems).
///
/// This structure has methods for building up an archive from scratch into any
/// arbitrary writer.
pub struct Builder<W: AsyncWrite + Unpin> {
    writer: W,
    started: bool,
}

impl<W: AsyncWrite + Unpin> Builder<W> {
    /// Create a new archive builder with the underlying writer object as the
    /// destination of all data written.
    pub fn new(writer: W) -> Builder<W> {
        Builder { writer, started: false }
    }

    /// Unwrap this archive builder, returning the underlying writer object.
    pub fn into_inner(self) -> Result<W> {
        Ok(self.writer)
    }

    /// Adds a new entry to this archive.
    pub async fn append<R: AsyncRead + Unpin>(
        &mut self,
        header: &Header,
        mut data: R,
    ) -> Result<()> {
        if !self.started {
            self.writer.write_all(GLOBAL_HEADER).await?;
            self.started = true;
        }
        header.write(&mut self.writer).await?;
        let actual_size = tokio::io::copy(&mut data, &mut self.writer).await?;
        if actual_size != header.size() {
            let msg = format!(
                "Wrong file size (header.size() = {}, actual \
                               size was {})",
                header.size(),
                actual_size
            );
            return Err(Error::new(ErrorKind::InvalidData, msg));
        }
        if actual_size % 2 != 0 {
            self.writer.write_all(b"\n").await?;
        }
        Ok(())
    }

    /// Adds a file on the local filesystem to this archive, using the file
    /// name as its identifier.
    pub async fn append_path<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> Result<()> {
        let name: &OsStr = path.as_ref().file_name().ok_or_else(|| {
            let msg = "Given path doesn't have a file name";
            Error::new(ErrorKind::InvalidInput, msg)
        })?;
        let identifier = osstr_to_bytes(name)?;
        let mut file = File::open(&path).await?;
        self.append_file_id(identifier, &mut file).await
    }

    /// Adds a file to this archive, with the given name as its identifier.
    pub async fn append_file(
        &mut self,
        name: &[u8],
        file: &mut File,
    ) -> Result<()> {
        self.append_file_id(name.to_vec(), file).await
    }

    async fn append_file_id(
        &mut self,
        id: Vec<u8>,
        file: &mut File,
    ) -> Result<()> {
        let metadata = file.metadata().await?;
        let header = Header::from_metadata(id, &metadata);
        self.append(&header, file).await
    }
}

// ========================================================================= //

/// A structure for building GNU-variant archives (the archive format typically
/// used on e.g. GNU/Linux and Windows systems).
///
/// This structure has methods for building up an archive from scratch into any
/// arbitrary writer.
pub struct GnuBuilder<W: AsyncWrite + Unpin> {
    writer: W,
    short_names: HashSet<Vec<u8>>,
    long_names: HashMap<Vec<u8>, usize>,
    name_table_size: usize,
    name_table_needs_padding: bool,
    started: bool,
}

impl<W: AsyncWrite + Unpin> GnuBuilder<W> {
    /// Create a new archive builder with the underlying writer object as the
    /// destination of all data written.  The `identifiers` parameter must give
    /// the complete list of entry identifiers that will be included in this
    /// archive.
    pub fn new(writer: W, identifiers: Vec<Vec<u8>>) -> GnuBuilder<W> {
        let mut short_names = HashSet::<Vec<u8>>::new();
        let mut long_names = HashMap::<Vec<u8>, usize>::new();
        let mut name_table_size: usize = 0;
        for identifier in identifiers.into_iter() {
            let length = identifier.len();
            if length > 15 {
                long_names.insert(identifier, name_table_size);
                name_table_size += length + 2;
            } else {
                short_names.insert(identifier);
            }
        }
        let name_table_needs_padding = !name_table_size.is_multiple_of(2);
        if name_table_needs_padding {
            name_table_size += 3; // ` /\n`
        }

        GnuBuilder {
            writer,
            short_names,
            long_names,
            name_table_size,
            name_table_needs_padding,
            started: false,
        }
    }

    /// Unwrap this archive builder, returning the underlying writer object.
    pub fn into_inner(self) -> Result<W> {
        Ok(self.writer)
    }

    /// Adds a new entry to this archive.
    pub async fn append<R: AsyncRead + Unpin>(
        &mut self,
        header: &Header,
        mut data: R,
    ) -> Result<()> {
        let is_long_name = header.identifier().len() > 15;
        let has_name = if is_long_name {
            self.long_names.contains_key(header.identifier())
        } else {
            self.short_names.contains(header.identifier())
        };
        if !has_name {
            let msg = format!(
                "Identifier {:?} was not in the list of \
                 identifiers passed to GnuBuilder::new()",
                String::from_utf8_lossy(header.identifier())
            );
            return Err(Error::new(ErrorKind::InvalidInput, msg));
        }

        if !self.started {
            self.writer.write_all(GLOBAL_HEADER).await?;
            if !self.long_names.is_empty() {
                self.writer
                    .write_all(
                        format!(
                            "{:<48}{:<10}`\n",
                            GNU_NAME_TABLE_ID, self.name_table_size
                        )
                        .as_bytes(),
                    )
                    .await?;
                let mut entries: Vec<(usize, &[u8])> = self
                    .long_names
                    .iter()
                    .map(|(id, &start)| (start, id.as_slice()))
                    .collect();
                entries.sort();
                for (_, id) in entries {
                    self.writer.write_all(id).await?;
                    self.writer.write_all(b"/\n").await?;
                }
                if self.name_table_needs_padding {
                    self.writer.write_all(b" /\n").await?;
                }
            }
            self.started = true;
        }

        header.write_gnu(&mut self.writer, &self.long_names).await?;
        let actual_size = tokio::io::copy(&mut data, &mut self.writer).await?;
        if actual_size != header.size() {
            let msg = format!(
                "Wrong file size (header.size() = {}, actual \
                               size was {})",
                header.size(),
                actual_size
            );
            return Err(Error::new(ErrorKind::InvalidData, msg));
        }
        if actual_size % 2 != 0 {
            self.writer.write_all(b"\n").await?;
        }

        Ok(())
    }

    /// Adds a file on the local filesystem to this archive, using the file
    /// name as its identifier.
    pub async fn append_path<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> Result<()> {
        let name: &OsStr = path.as_ref().file_name().ok_or_else(|| {
            let msg = "Given path doesn't have a file name";
            Error::new(ErrorKind::InvalidInput, msg)
        })?;
        let identifier = osstr_to_bytes(name)?;
        let mut file = File::open(&path).await?;
        self.append_file_id(identifier, &mut file).await
    }

    /// Adds a file to this archive, with the given name as its identifier.
    pub async fn append_file(
        &mut self,
        name: &[u8],
        file: &mut File,
    ) -> Result<()> {
        self.append_file_id(name.to_vec(), file).await
    }

    async fn append_file_id(
        &mut self,
        id: Vec<u8>,
        file: &mut File,
    ) -> Result<()> {
        let metadata = file.metadata().await?;
        let header = Header::from_metadata(id, &metadata);
        self.append(&header, file).await
    }
}

#[cfg(unix)]
fn osstr_to_bytes(string: &OsStr) -> Result<Vec<u8>> {
    Ok(string.as_bytes().to_vec())
}

#[cfg(not(unix))]
fn osstr_to_bytes(string: &OsStr) -> Result<Vec<u8>> {
    let utf8: &str = string.to_str().ok_or_else(|| {
        Error::new(ErrorKind::InvalidData, "Non-UTF8 file name")
    })?;
    Ok(utf8.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::{Builder, GnuBuilder, Header};
    use std::str;

    #[tokio::test]
    async fn build_common_archive() {
        let mut builder = Builder::new(Vec::new());
        let mut header1 = Header::new(b"foo.txt".to_vec(), 7);
        header1.set_mtime(1487552916);
        header1.set_uid(501);
        header1.set_gid(20);
        header1.set_mode(0o100644);
        builder.append(&header1, "foobar\n".as_bytes()).await.unwrap();
        let header2 = Header::new(b"baz.txt".to_vec(), 4);
        builder.append(&header2, "baz\n".as_bytes()).await.unwrap();
        let actual = builder.into_inner().unwrap();
        let expected = "\
        !<arch>\n\
        foo.txt         1487552916  501   20    100644  7         `\n\
        foobar\n\n\
        baz.txt         0           0     0     0       4         `\n\
        baz\n";
        assert_eq!(str::from_utf8(&actual).unwrap(), expected);
    }

    #[tokio::test]
    async fn build_common_archive_with_uid_gid_overflow() {
        let mut builder = Builder::new(Vec::new());
        let mut header1 = Header::new(b"foo.txt".to_vec(), 7);
        header1.set_mtime(1487552916);
        header1.set_uid(1234567);
        header1.set_gid(7654321);
        header1.set_mode(0o100644);
        builder.append(&header1, "foobar\n".as_bytes()).await.unwrap();
        let header2 = Header::new(b"baz.txt".to_vec(), 4);
        builder.append(&header2, "baz\n".as_bytes()).await.unwrap();
        let actual = builder.into_inner().unwrap();
        let expected = "\
        !<arch>\n\
        foo.txt         1487552916  123456765432100644  7         `\n\
        foobar\n\n\
        baz.txt         0           0     0     0       4         `\n\
        baz\n";
        assert_eq!(str::from_utf8(&actual).unwrap(), expected);
    }

    #[tokio::test]
    async fn build_bsd_archive_with_long_filenames() {
        let mut builder = Builder::new(Vec::new());
        let mut header1 = Header::new(b"short".to_vec(), 1);
        header1.set_identifier(b"this_is_a_very_long_filename.txt".to_vec());
        header1.set_mtime(1487552916);
        header1.set_uid(501);
        header1.set_gid(20);
        header1.set_mode(0o100644);
        header1.set_size(7);
        builder.append(&header1, "foobar\n".as_bytes()).await.unwrap();
        let header2 = Header::new(
            b"and_this_is_another_very_long_filename.txt".to_vec(),
            4,
        );
        builder.append(&header2, "baz\n".as_bytes()).await.unwrap();
        let actual = builder.into_inner().unwrap();
        let expected = "\
        !<arch>\n\
        #1/32           1487552916  501   20    100644  39        `\n\
        this_is_a_very_long_filename.txtfoobar\n\n\
        #1/44           0           0     0     0       48        `\n\
        and_this_is_another_very_long_filename.txt\x00\x00baz\n";
        assert_eq!(str::from_utf8(&actual).unwrap(), expected);
    }

    #[tokio::test]
    async fn build_bsd_archive_with_space_in_filename() {
        let mut builder = Builder::new(Vec::new());
        let header = Header::new(b"foo bar".to_vec(), 4);
        builder.append(&header, "baz\n".as_bytes()).await.unwrap();
        let actual = builder.into_inner().unwrap();
        let expected = "\
        !<arch>\n\
        #1/8            0           0     0     0       12        `\n\
        foo bar\x00baz\n";
        assert_eq!(str::from_utf8(&actual).unwrap(), expected);
    }

    #[tokio::test]
    async fn build_gnu_archive() {
        let names = vec![b"baz.txt".to_vec(), b"foo.txt".to_vec()];
        let mut builder = GnuBuilder::new(Vec::new(), names);
        let mut header1 = Header::new(b"foo.txt".to_vec(), 7);
        header1.set_mtime(1487552916);
        header1.set_uid(501);
        header1.set_gid(20);
        header1.set_mode(0o100644);
        builder.append(&header1, "foobar\n".as_bytes()).await.unwrap();
        let header2 = Header::new(b"baz.txt".to_vec(), 4);
        builder.append(&header2, "baz\n".as_bytes()).await.unwrap();
        let actual = builder.into_inner().unwrap();
        let expected = "\
        !<arch>\n\
        foo.txt/        1487552916  501   20    100644  7         `\n\
        foobar\n\n\
        baz.txt/        0           0     0     0       4         `\n\
        baz\n";
        assert_eq!(str::from_utf8(&actual).unwrap(), expected);
    }

    #[tokio::test]
    async fn build_gnu_archive_with_long_filenames() {
        let names = vec![
            b"this_is_a_very_long_filename.txt".to_vec(),
            b"and_this_is_another_very_long_filename.txt".to_vec(),
        ];
        let mut builder = GnuBuilder::new(Vec::new(), names);
        let mut header1 = Header::new(b"short".to_vec(), 1);
        header1.set_identifier(b"this_is_a_very_long_filename.txt".to_vec());
        header1.set_mtime(1487552916);
        header1.set_uid(501);
        header1.set_gid(20);
        header1.set_mode(0o100644);
        header1.set_size(7);
        builder.append(&header1, "foobar\n".as_bytes()).await.unwrap();
        let header2 = Header::new(
            b"and_this_is_another_very_long_filename.txt".to_vec(),
            4,
        );
        builder.append(&header2, "baz\n".as_bytes()).await.unwrap();
        let actual = builder.into_inner().unwrap();
        let expected = "\
        !<arch>\n\
        //                                              78        `\n\
        this_is_a_very_long_filename.txt/\n\
        and_this_is_another_very_long_filename.txt/\n\
        /0              1487552916  501   20    100644  7         `\n\
        foobar\n\n\
        /34             0           0     0     0       4         `\n\
        baz\n";
        assert_eq!(str::from_utf8(&actual).unwrap(), expected);
    }

    #[tokio::test]
    async fn build_gnu_archive_with_space_in_filename() {
        let names = vec![b"foo bar".to_vec()];
        let mut builder = GnuBuilder::new(Vec::new(), names);
        let header = Header::new(b"foo bar".to_vec(), 4);
        builder.append(&header, "baz\n".as_bytes()).await.unwrap();
        let actual = builder.into_inner().unwrap();
        let expected = "\
        !<arch>\n\
        foo bar/        0           0     0     0       4         `\n\
        baz\n";
        assert_eq!(str::from_utf8(&actual).unwrap(), expected);
    }

    #[tokio::test]
    #[should_panic(
        expected = "Identifier \\\"bar\\\" was not in the list of \
                               identifiers passed to GnuBuilder::new()"
    )]
    async fn build_gnu_archive_with_unexpected_identifier() {
        let names = vec![b"foo".to_vec()];
        let mut builder = GnuBuilder::new(Vec::new(), names);
        let header = Header::new(b"bar".to_vec(), 4);
        builder.append(&header, "baz\n".as_bytes()).await.unwrap();
    }
}
