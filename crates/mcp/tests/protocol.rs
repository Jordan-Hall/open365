//! Protocol conformance (MCP 2026-07-28 plus the legacy handshake), resources and the read
//! tools added with full command coverage.

mod common;

use common::{Rig, Scratch, s};
use serde_json::{Value, json};

const META: &str = "io.modelcontextprotocol/protocolVersion";

fn modern(extra: Value) -> Value {
    let mut params = extra;
    params["_meta"] = json!({META: "2026-07-28"});
    params
}

#[test]
fn modern_requests_get_the_2026_07_28_result_shape() {
    let scratch = Scratch::new("modern");
    let mut r = Rig::new(&scratch, &[], false);
    let d = r.rpc("server/discover", json!({}));
    let res = &d["result"];
    assert_eq!(res["resultType"], "complete");
    assert_eq!(res["supportedVersions"][0], "2026-07-28");
    assert!(
        res["capabilities"]["tools"].is_object() && res["capabilities"]["resources"].is_object()
    );
    assert_eq!(
        res["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "open365-mcp"
    );

    let list = r.rpc("tools/list", modern(json!({})));
    assert_eq!(list["result"]["resultType"], "complete");
    assert!(list["result"]["ttlMs"].is_u64() && list["result"]["cacheScope"] == "public");

    // ping was removed in 2026-07-28 but legacy sessions keep it.
    assert_eq!(r.rpc("ping", modern(json!({})))["error"]["code"], -32601);
    assert_eq!(r.rpc("ping", json!({}))["result"], json!({}));

    // An unknown version is a modern error naming what the server supports.
    let bad = r.rpc("tools/list", json!({"_meta": {META: "1900-01-01"}}));
    assert_eq!(bad["error"]["code"], -32022);
    assert_eq!(bad["error"]["data"]["requested"], "1900-01-01");
    assert!(
        bad["error"]["data"]["supported"]
            .as_array()
            .is_some_and(|v| v.len() >= 2)
    );

    // A legacy list has none of the new fields.
    let legacy = r.rpc("tools/list", json!({}));
    assert!(legacy["result"].get("resultType").is_none());
}

#[test]
fn legacy_handshake_still_works_for_every_older_revision() {
    let scratch = Scratch::new("legacy");
    let mut r = Rig::new(&scratch, &[], false);
    for v in ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"] {
        let init = r.rpc("initialize", json!({"protocolVersion": v}));
        assert_eq!(init["result"]["protocolVersion"], v);
        assert!(init["result"]["capabilities"]["resources"].is_object());
    }
}

#[test]
fn resources_list_templates_and_read() {
    let scratch = Scratch::new("res");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let mut r = Rig::new(&scratch, &[], false);
    let list = r.rpc("resources/list", modern(json!({})));
    let uris: Vec<&str> = list["result"]["resources"]
        .as_array()
        .expect("resources")
        .iter()
        .map(|x| x["uri"].as_str().expect("uri"))
        .collect();
    assert_eq!(uris, ["open365://commands", "open365://session"]);
    let templ = r.rpc("resources/templates/list", json!({}));
    assert!(
        templ["result"]["resourceTemplates"]
            .as_array()
            .is_some_and(|t| t.len() >= 6)
    );

    let d = r.open(&docx);
    let x = r.open(&xlsx);
    let read = |r: &mut Rig, uri: &str| -> Value {
        let reply = r.rpc("resources/read", modern(json!({"uri": uri})));
        assert!(reply["error"].is_null(), "{uri}: {reply}");
        assert_eq!(reply["result"]["contents"][0]["uri"], uri);
        serde_json::from_str(
            reply["result"]["contents"][0]["text"]
                .as_str()
                .expect("text"),
        )
        .expect("json")
    };
    assert_eq!(read(&mut r, "open365://session")["scopes"], json!(["read"]));
    assert!(read(&mut r, &format!("open365://document/{d}/outline"))["headings"].is_array());
    assert!(read(&mut r, &format!("open365://document/{d}/text/0"))["paragraphs"].is_array());
    assert!(read(&mut r, &format!("open365://document/{d}/styles"))["styles"].is_array());
    assert!(read(&mut r, &format!("open365://document/{d}/structure"))["parts"].is_array());
    assert!(read(&mut r, &format!("open365://document/{x}/cells/0/A1:B2"))["cells"].is_array());
    assert!(read(&mut r, &format!("open365://document/{x}/formulas/0"))["formulas"].is_array());
    let missing = r.rpc(
        "resources/read",
        json!({"uri": "open365://document/doc99/outline"}),
    );
    assert_eq!(missing["error"]["code"], -32602);
}

#[test]
fn styles_formulas_and_recalc_read_a_workbook() {
    let scratch = Scratch::new("readtools");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = Rig::new(&scratch, &[], false);
    let x = r.open(&xlsx);
    let f = r.ok("read_formulas", json!({"document": x, "sheet": 0}));
    assert!(f["count"].as_u64().unwrap_or(0) > 0, "{f}");
    let first = f["formulas"][0]["ref"].as_str().expect("ref").to_owned();
    let st = r.ok(
        "get_styles",
        json!({"document": x, "sheet": 0, "range": first}),
    );
    assert!(st["cells"][0]["font"]["name"].is_string(), "{st}");
    assert!(st["cell_styles"].as_array().is_some_and(|a| !a.is_empty()));
    let rc = r.ok("recalc", json!({"document": x, "sheet": 0}));
    assert_eq!(rc["saved"], false);
    assert_eq!(rc["formulas_checked"], f["count"], "{rc}");
    assert_eq!(rc["stale"], 0, "a clean fixture has no stale cells: {rc}");

    let d = r.open(&docx);
    let ds = r.ok("get_styles", json!({"document": d}));
    assert!(ds["count"].as_u64().unwrap_or(0) > 0);
    let (v, err) = r.call("recalc", json!({"document": d, "sheet": 0}));
    assert!(err && v["status"] == "invalid");
    let _ = s;
}
