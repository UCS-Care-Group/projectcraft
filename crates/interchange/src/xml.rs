//! A guarded XML walk over `quick-xml`: DTDs refused (so no entity can be declared, and no
//! billion laughs), every entity other than XML's five and character references refused, nesting
//! capped at [`MAX_DEPTH`], the file at [`MAX_IMPORT_BYTES`], UTF-8 only. Each element's opening
//! and closing goes to a [`Visitor`] with the local names of the element and its ancestors.

use quick_xml::Reader;
use quick_xml::events::{BytesRef, BytesStart, Event};

use crate::{ImportError, MAX_DEPTH, MAX_IMPORT_BYTES};

pub(crate) trait Visitor {
    /// An element opens. `path` holds its local name and its ancestors', the root first.
    fn open(&mut self, path: &[String]) -> Result<(), ImportError>;
    /// It closes, with the text directly inside it (entities resolved).
    fn close(&mut self, path: &[String], text: &str) -> Result<(), ImportError>;
}

/// The file as text: refused past [`MAX_IMPORT_BYTES`] or when not UTF-8; a byte-order mark is skipped.
pub(crate) fn text_of(bytes: &[u8]) -> Result<&str, ImportError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::TooLarge);
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    std::str::from_utf8(bytes).map_err(|_| ImportError::NotUtf8)
}

fn xml_error(r: &Reader<&[u8]>, e: impl std::fmt::Display) -> ImportError {
    ImportError::Xml(format!("{e} (at byte {})", r.error_position()))
}

fn local_name(e: &BytesStart) -> Result<String, ImportError> {
    std::str::from_utf8(e.local_name().as_ref()).map(str::to_owned).map_err(|_| ImportError::NotUtf8)
}

/// The root element's local name and its default namespace (`xmlns`), if it has one.
pub(crate) fn root(text: &str) -> Result<(String, Option<String>), ImportError> {
    let mut r = Reader::from_str(text);
    loop {
        match r.read_event().map_err(|e| xml_error(&r, e))? {
            Event::Start(e) | Event::Empty(e) => {
                let mut ns = None;
                for a in e.attributes() {
                    let a = a.map_err(|e| xml_error(&r, e))?;
                    if a.key.as_ref() == b"xmlns" {
                        ns = Some(String::from_utf8_lossy(&a.value).into_owned());
                    }
                }
                return Ok((local_name(&e)?, ns));
            }
            Event::DocType(_) => return Err(ImportError::Dtd),
            Event::Eof => return Err(ImportError::NotMspdi("nothing".into())),
            Event::Text(t) if t.iter().all(u8::is_ascii_whitespace) => {}
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) | Event::End(_) => {
                return Err(ImportError::Xml("text before the first element".into()));
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
        }
    }
}

/// An entity reference's character: `&lt;`, `&gt;`, `&amp;`, `&apos;`, `&quot;` and character
/// references (`&#38;`, `&#x26;`); anything else is refused.
fn resolve(r: &BytesRef) -> Result<char, ImportError> {
    if r.is_char_ref() {
        return match r.resolve_char_ref() {
            Ok(Some(c)) => Ok(c),
            _ => Err(ImportError::Xml("a character reference that isn't a character".into())),
        };
    }
    let name = r.decode().map_err(|_| ImportError::NotUtf8)?;
    match name.as_ref() {
        "lt" => Ok('<'),
        "gt" => Ok('>'),
        "amp" => Ok('&'),
        "apos" => Ok('\''),
        "quot" => Ok('"'),
        other => Err(ImportError::Entity(other.chars().take(40).collect())),
    }
}

/// Walks the whole document, element by element.
pub(crate) fn walk(text: &str, v: &mut impl Visitor) -> Result<(), ImportError> {
    let mut r = Reader::from_str(text);
    let mut path: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut rooted = false;
    loop {
        let event = r.read_event().map_err(|e| xml_error(&r, e))?;
        match event {
            Event::Start(e) | Event::Empty(e) if path.is_empty() && rooted => {
                let name = local_name(&e)?;
                return Err(ImportError::Xml(format!("a second root element `{}`", name.chars().take(40).collect::<String>())));
            }
            Event::Start(ref e) | Event::Empty(ref e) => {
                if path.len() >= MAX_DEPTH {
                    return Err(ImportError::TooDeep);
                }
                rooted = true;
                path.push(local_name(e)?);
                buf.clear();
                v.open(&path)?;
                if matches!(event, Event::Empty(_)) {
                    v.close(&path, "")?;
                    path.pop();
                }
            }
            Event::End(_) => {
                v.close(&path, &buf)?;
                path.pop();
                buf.clear();
            }
            Event::Text(t) if !path.is_empty() => buf.push_str(&t.xml10_content().map_err(|_| ImportError::NotUtf8)?),
            Event::CData(t) if !path.is_empty() => buf.push_str(&t.decode().map_err(|_| ImportError::NotUtf8)?),
            Event::GeneralRef(g) => {
                let c = resolve(&g)?;
                if !path.is_empty() {
                    buf.push(c);
                }
            }
            Event::DocType(_) => return Err(ImportError::Dtd),
            Event::Eof => break,
            Event::Text(_) | Event::CData(_) | Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
        }
    }
    match path.last() {
        Some(open) => Err(ImportError::Xml(format!("the file ends inside `{}`", open.chars().take(40).collect::<String>()))),
        None if !rooted => Err(ImportError::NotMspdi("nothing".into())),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Log(Vec<String>);

    impl Visitor for Log {
        fn open(&mut self, path: &[String]) -> Result<(), ImportError> {
            self.0.push(format!("+{}", path.join("/")));
            Ok(())
        }
        fn close(&mut self, path: &[String], text: &str) -> Result<(), ImportError> {
            self.0.push(format!("-{}={}", path.join("/"), text.trim()));
            Ok(())
        }
    }

    fn log(text: &str) -> Result<Vec<String>, ImportError> {
        let mut l = Log::default();
        walk(text, &mut l)?;
        Ok(l.0)
    }

    #[test]
    fn walks_elements_with_their_paths_and_text() {
        let got = log("<?xml version=\"1.0\"?><!-- c --><a xmlns=\"x\"><b>1 &amp; 2 &#x41;&#66;</b><c/><p:d xmlns:p=\"y\"><![CDATA[<z>]]></p:d></a>")
            .unwrap();
        assert_eq!(got, ["+a", "+a/b", "-a/b=1 & 2 AB", "+a/c", "-a/c=", "+a/d", "-a/d=<z>", "-a="]);
        assert_eq!(
            root(" <!--x--> <Project xmlns=\"http://schemas.microsoft.com/project\">").unwrap(),
            ("Project".into(), Some("http://schemas.microsoft.com/project".into()))
        );
    }

    #[test]
    fn dtds_and_entities_are_refused() {
        let laughs = "<?xml version=\"1.0\"?><!DOCTYPE lolz [<!ENTITY lol \"lol\"><!ENTITY lol2 \"&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;\">]><Project>&lol2;</Project>";
        assert_eq!(log(laughs).unwrap_err(), ImportError::Dtd);
        assert_eq!(root(laughs).unwrap_err(), ImportError::Dtd);
        assert_eq!(log("<a>&lol;</a>").unwrap_err(), ImportError::Entity("lol".into()));
        assert!(log("<a>&#0;</a>").is_err());
        assert_eq!(log("<a><b/><!DOCTYPE x></a>").unwrap_err(), ImportError::Dtd);
    }

    #[test]
    fn depth_is_capped_and_bad_shapes_refused() {
        let ok = "<a>".repeat(MAX_DEPTH) + &"</a>".repeat(MAX_DEPTH);
        assert!(log(&ok).is_ok());
        let deep = "<a>".repeat(MAX_DEPTH + 1) + &"</a>".repeat(MAX_DEPTH + 1);
        assert_eq!(log(&deep).unwrap_err(), ImportError::TooDeep);
        // A depth bomb never closes: refused at the cap, long before the end.
        let bomb = "<a>".repeat(5_000_000);
        let t = std::time::Instant::now();
        assert_eq!(log(&bomb).unwrap_err(), ImportError::TooDeep);
        assert!(t.elapsed() < std::time::Duration::from_secs(1));
        assert!(matches!(log("<a><b></a>"), Err(ImportError::Xml(_))));
        assert!(matches!(log("<a>"), Err(ImportError::Xml(_))));
        assert!(matches!(log("<a/><b/>"), Err(ImportError::Xml(_))));
        assert!(matches!(log(""), Err(ImportError::NotMspdi(_))));
        assert!(matches!(log("</a>"), Err(ImportError::Xml(_))));
    }

    #[test]
    fn size_and_encoding_are_checked() {
        assert_eq!(text_of(b"\xEF\xBB\xBF<a/>").unwrap(), "<a/>");
        assert_eq!(text_of(b"\xFF\xFE<\0a\0/\0>\0").unwrap_err(), ImportError::NotUtf8);
        assert_eq!(text_of(&vec![b' '; MAX_IMPORT_BYTES + 1]).unwrap_err(), ImportError::TooLarge);
    }
}
