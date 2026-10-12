//! Pivot table creation hook: the caller supplies the part bytes (the pivot crate produces them) and
//! this wires the cache definition, the records, the table and their relationships and content types.

use open365_opc::TargetMode;

use super::{MAIN, R_NS, content_type, rel, relative_target};
use crate::XlsxError;
use crate::workbook::Workbook;
use crate::xml::{self, Attr, RawXml};

/// The three parts of a pivot table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotSpec {
    /// `pivotCacheDefinition` bytes. Its `r:id` (when it has records) must name the records
    /// relationship; it is added as `rId1` when absent.
    pub cache_definition: Vec<u8>,
    /// `pivotCacheRecords` bytes, when the cache is saved with its records.
    pub cache_records: Option<Vec<u8>>,
    /// `pivotTableDefinition` bytes. Its `cacheId` is set to the id this call assigns.
    pub table_definition: Vec<u8>,
}

/// What [`Workbook::add_pivot_table`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotInsert {
    pub cache_id: u32,
    pub cache_definition_part: String,
    pub cache_records_part: Option<String>,
    pub pivot_table_part: String,
}

fn check_root(bytes: &[u8], local: &str) -> Result<xml::ParsedXml, XlsxError> {
    let doc = xml::parse(bytes)?;
    if doc.root.head.local_name() != local {
        return Err(XlsxError::UnsupportedEdit(format!(
            "the part must be a {local} document"
        )));
    }
    Ok(doc)
}

impl Workbook {
    /// Adds a pivot table to worksheet `sheet` from caller-supplied parts: `xl/pivotCache/...`,
    /// `xl/pivotTables/pivotTableN.xml`, the workbook `pivotCaches` entry, the relationships between
    /// them and the content types, as Excel names them.
    ///
    /// # Errors
    /// The sheet is not a worksheet or a part is not the document it should be.
    pub fn add_pivot_table(
        &mut self,
        sheet: usize,
        spec: &PivotSpec,
    ) -> Result<PivotInsert, XlsxError> {
        self.require_transitional()?;
        let sheet_part = self
            .sheet_part(sheet)
            .cloned()
            .ok_or(XlsxError::NotAWorksheet(sheet))?;
        let mut cache = check_root(&spec.cache_definition, "pivotCacheDefinition")?;
        let records = match &spec.cache_records {
            Some(b) => Some(check_root(b, "pivotCacheRecords")?),
            None => None,
        };
        let mut table = check_root(&spec.table_definition, "pivotTableDefinition")?;
        self.edited()?;
        let wb_part = self.workbook_part.clone();

        let cache_id = self
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("pivotCaches"))
            .map_or(0, |c| {
                c.elements()
                    .filter_map(|p| {
                        p.head
                            .attr("cacheId")
                            .and_then(|v| v.trim().parse::<u32>().ok())
                    })
                    .max()
                    .unwrap_or(0)
            })
            + 1;

        // Cache definition (with its records relationship).
        let def_name = self.next_part_name("xl/pivotCache", "pivotCacheDefinition", "xml");
        let n = def_name
            .trim_start_matches("xl/pivotCache/pivotCacheDefinition")
            .trim_end_matches(".xml")
            .to_owned();
        let rec_name = format!("xl/pivotCache/pivotCacheRecords{n}.xml");
        if records.is_some() && cache.root.head.attr_r("id").is_none() {
            if !cache.root.head.attrs.iter().any(|a| a.name == "xmlns:r") {
                cache.root.head.attrs.push(Attr::plain("xmlns:r", R_NS));
            }
            cache.root.head.attrs.push(Attr {
                name: "r:id".to_owned(),
                ns: Some(R_NS.into()),
                value: "rId1".to_owned(),
            });
        }
        let def = self.add_part(
            &def_name,
            content_type::PIVOT_CACHE_DEFINITION,
            cache.to_bytes(),
        )?;
        let mut rec_part = None;
        if let Some(rec) = &records {
            let part =
                self.add_part(&rec_name, content_type::PIVOT_CACHE_RECORDS, rec.to_bytes())?;
            let want = cache.root.head.attr_r("id").unwrap_or("rId1").to_owned();
            self.add_relationship_with_id(
                &def,
                &want,
                rel::PIVOT_CACHE_RECORDS,
                &relative_target(Some(&def), part.as_str()),
                false,
            )?;
            rec_part = Some(part.to_string());
        }
        let wb_rid = self.add_relationship(
            Some(&wb_part),
            rel::PIVOT_CACHE_DEFINITION,
            &relative_target(Some(&wb_part), def.as_str()),
            TargetMode::Internal,
        )?;

        // `pivotCaches` in the workbook.
        let mut caches = self
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("pivotCaches"))
            .cloned()
            .map_or_else(
                || self.workbook_fragment(&format!("<pivotCaches xmlns=\"{MAIN}\"/>")),
                Ok,
            )?;
        let entry = self.workbook_fragment(&format!(
            "<pivotCache xmlns=\"{MAIN}\" xmlns:r=\"{R_NS}\" cacheId=\"{cache_id}\" r:id=\"{wb_rid}\"/>"
        ))?;
        caches.children.push(RawXml::Element(entry));
        self.put_workbook_child("pivotCaches", Some(caches));

        // The table.
        table.root.head.set_attr("cacheId", &cache_id.to_string());
        let table_name = self.next_part_name("xl/pivotTables", "pivotTable", "xml");
        let tp = self.add_part(&table_name, content_type::PIVOT_TABLE, table.to_bytes())?;
        self.add_relationship(
            Some(&sheet_part),
            rel::PIVOT_TABLE,
            &relative_target(Some(&sheet_part), tp.as_str()),
            TargetMode::Internal,
        )?;
        self.add_relationship(
            Some(&tp),
            rel::PIVOT_CACHE_DEFINITION,
            &relative_target(Some(&tp), def.as_str()),
            TargetMode::Internal,
        )?;
        Ok(PivotInsert {
            cache_id,
            cache_definition_part: def.to_string(),
            cache_records_part: rec_part,
            pivot_table_part: tp.to_string(),
        })
    }
}
