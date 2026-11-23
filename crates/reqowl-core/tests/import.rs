//! Imports each fixture into a temp dir, then reads the result back with the
//! normal page/request parsers and workspace loader.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqowl_core::env::Workspace;
use reqowl_core::import::{self, Format};
use reqowl_core::request::{self, Kind, RequestSpec};
use reqowl_core::{Error, page};

fn fixture(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn import_fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
    let ws = import::detect_and_parse(&fixture(name)).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("ws");
    import::write_workspace(&ws, &root).unwrap();
    (tmp, root)
}

/// Every page under `root` (relative path, `/`-separated) with its parsed
/// request blocks. Panics if any block fails to parse.
fn load_pages(root: &Path) -> BTreeMap<String, Vec<RequestSpec>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<RequestSpec>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().unwrap() != "env" {
                    walk(root, &path, out);
                }
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            let specs = page::parse(&text)
                .blocks
                .iter()
                .map(|b| {
                    request::parse(&b.content, b.content_line)
                        .unwrap_or_else(|e| panic!("{rel}: block {} does not parse: {e}\n{text}", b.index))
                })
                .collect();
            out.insert(rel, specs);
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn only<'a>(pages: &'a BTreeMap<String, Vec<RequestSpec>>, path: &str) -> &'a RequestSpec {
    let specs = pages
        .get(path)
        .unwrap_or_else(|| panic!("no page {path}; have {:?}", pages.keys()));
    assert_eq!(specs.len(), 1, "{path}");
    &specs[0]
}

fn order(root: &Path, dir: &str) -> Vec<String> {
    fs::read_to_string(root.join(dir).join(".order"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

#[test]
fn postman_collection() {
    assert_eq!(
        import::detect(&fixture("postman_collection_v21.json")).unwrap(),
        Format::PostmanCollection
    );
    let (_tmp, root) = import_fixture("postman_collection_v21.json");
    let pages = load_pages(&root);

    let ws = Workspace::open(&root).unwrap();
    assert_eq!(ws.config.name.as_deref(), Some("Shop API"));
    let vars = ws.defaults().unwrap();
    assert_eq!(vars["base_url"], "https://shop.example.com/api");
    assert_eq!(vars["retries"], "3");
    assert!(!vars.contains_key("old"), "disabled variable");
    assert!(ws.env_names().unwrap().is_empty());

    assert_eq!(
        order(&root, ""),
        [
            "About",
            "Auth",
            "Orders",
            "Search products (GraphQL)",
            "Health check",
            "Report daily sales"
        ]
    );
    assert_eq!(order(&root, "Orders"), ["List orders", "Admin"]);
    assert_eq!(order(&root, "Orders/Admin"), ["Delete order", "Upload invoice"]);
    assert!(read(&root, "Auth.md").contains("Login and token refresh."));
    assert!(read(&root, ".gitignore").contains("env/*.local.toml"));

    // Raw JSON body, folder `noauth` overrides the collection bearer.
    let login = only(&pages, "Auth/Login.md");
    assert_eq!(login.name(), Some("Login"));
    assert_eq!(
        (login.method.as_str(), login.url.as_str()),
        ("POST", "{{base_url}}/auth/login")
    );
    assert_eq!(login.header("content-type"), Some("application/json"));
    assert_eq!(login.directives.auth, None);
    assert!(login.body.as_deref().unwrap().contains("\"user\": \"{{username}}\""));
    let login_md = read(&root, "Auth/Login.md");
    assert!(login_md.contains("Postman script not imported"));
    assert!(login_md.contains("```js\npm.test("));
    assert!(login_md.contains("Returns a session token."));

    // Basic auth on the request, urlencoded body without the disabled field.
    let refresh = only(&pages, "Auth/Refresh token.md");
    assert_eq!(refresh.name(), Some("Refresh_token"));
    assert_eq!(
        refresh.directives.auth.as_deref(),
        Some("basic {{client_id}} {{client_secret}}")
    );
    assert_eq!(
        refresh.header("Content-Type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        refresh.body.as_deref(),
        Some("grant_type=refresh_token&refresh_token={{refresh_token}}&scope=orders%3Aread%20orders%3Awrite")
    );

    // Collection bearer inherited; disabled header skipped; folder variable in front matter.
    let list = only(&pages, "Orders/List orders.md");
    assert_eq!(list.method, "GET");
    assert_eq!(list.url, "{{base_url}}/orders?limit={{page_size}}&status=open");
    assert_eq!(list.directives.auth.as_deref(), Some("bearer {{token}}"));
    assert_eq!(list.headers.len(), 1);
    assert!(list.header("x-debug").is_none());
    let fm = page::parse(&read(&root, "Orders/List orders.md")).front_matter;
    assert_eq!(fm["page_size"], "20");

    // Nested folder apikey auth, path variable filled.
    let del = only(&pages, "Orders/Admin/Delete order.md");
    assert_eq!(
        (del.method.as_str(), del.url.as_str()),
        ("DELETE", "{{base_url}}/orders/42")
    );
    assert_eq!(del.directives.auth.as_deref(), Some("apikey X-Admin-Key {{admin_key}}"));

    // Explicit "inherit"; text form fields kept, file field noted.
    let upload = only(&pages, "Orders/Admin/Upload invoice.md");
    assert_eq!(
        upload.directives.auth.as_deref(),
        Some("apikey X-Admin-Key {{admin_key}}")
    );
    assert!(
        upload
            .header("content-type")
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    let body = upload.body.as_deref().unwrap();
    assert!(body.contains("name=\"note\"\n\npaid by card"));
    assert!(!body.contains("invoice-42.pdf"));
    assert!(read(&root, "Orders/Admin/Upload invoice.md").contains("file fields are not imported"));

    // GraphQL body: query, blank line, variables.
    let gql = only(&pages, "Search products (GraphQL).md");
    assert_eq!(gql.kind, Kind::GraphQl);
    assert_eq!(gql.url, "{{base_url}}/graphql");
    let (q, v) = gql.body.as_deref().unwrap().rsplit_once("\n\n").unwrap();
    assert!(q.starts_with("query Search($q: String!"));
    let v: serde_json::Value = serde_json::from_str(v).unwrap();
    assert_eq!(v["q"], "lamp");
    assert!(read(&root, "Search products (GraphQL).md").contains("Full-text product search."));

    // Request given as a plain URL string.
    let health = only(&pages, "Health check.md");
    assert_eq!(
        (health.method.as_str(), health.url.as_str()),
        ("GET", "https://status.example.com/health")
    );

    // API key in the query: noted, not applied. Dynamic variables mapped.
    let report = only(&pages, "Report daily sales.md");
    assert_eq!(report.directives.auth, None);
    assert_eq!(
        report.url,
        "{{base_url}}/reports/daily?day={{$isoTimestamp}}&id={{$uuid}}"
    );
    let report_md = read(&root, "Report daily sales.md");
    assert!(report_md.starts_with("# Report: daily sales\n"));
    assert!(report_md.contains("api_key={{report_key}}"));

    // Collection description and script.
    assert!(read(&root, "About.md").contains("Endpoints for the demo shop."));
    assert!(read(&root, "About.md").contains("X-Request-Id"));
    assert!(pages["About.md"].is_empty());
    // Folder pages hold no blocks.
    assert!(pages["Orders.md"].is_empty() && pages["Orders/Admin.md"].is_empty());
}

#[test]
fn postman_environment() {
    assert_eq!(
        import::detect(&fixture("postman_environment.json")).unwrap(),
        Format::PostmanEnvironment
    );
    let (_tmp, root) = import_fixture("postman_environment.json");
    let ws = Workspace::open(&root).unwrap();
    assert_eq!(ws.config.default_env.as_deref(), Some("Shop Staging"));
    assert_eq!(ws.env_names().unwrap(), ["Shop Staging"]);
    let env = ws.load_env("Shop Staging").unwrap();
    assert_eq!(env.vars["base_url"], "https://staging.shop.example.com/api");
    assert_eq!(env.vars.len(), 2);
    assert_eq!(env.secrets["client_secret"], "s3cr3t-staging");
    assert!(env.secrets.contains_key("token"));
    assert!(!env.vars.contains_key("legacy_host") && !env.secrets.contains_key("legacy_host"));
    assert!(root.join("env/Shop Staging.local.toml").is_file());
}

#[test]
fn insomnia_v4() {
    assert_eq!(
        import::detect(&fixture("insomnia_v4.json")).unwrap(),
        Format::InsomniaV4
    );
    let (_tmp, root) = import_fixture("insomnia_v4.json");
    let pages = load_pages(&root);

    let ws = Workspace::open(&root).unwrap();
    assert_eq!(ws.config.name.as_deref(), Some("Petstore"));
    let vars = ws.defaults().unwrap();
    assert_eq!(vars["base_url"], "{{scheme}}://{{host}}/v2");
    assert_eq!(vars["api.version"], "2");
    assert_eq!(ws.env_names().unwrap(), ["Local secrets", "Production"]);
    assert_eq!(ws.config.default_env.as_deref(), Some("Production"));
    assert_eq!(ws.load_env("Production").unwrap().vars["host"], "petstore.example.com");
    let private = ws.load_env("Local secrets").unwrap();
    assert!(private.vars.is_empty());
    assert_eq!(private.secrets["client_secret"], "dev-secret");

    // metaSortKey order, not file order.
    assert_eq!(
        order(&root, ""),
        [
            "About",
            "Get token",
            "Pets",
            "Pet by id (GraphQL)",
            "Who am I",
            "Live feed"
        ]
    );
    assert_eq!(order(&root, "Pets"), ["List pets", "Create pet", "Upload photo"]);

    let list = only(&pages, "Pets/List pets.md");
    assert_eq!(list.url, "{{base_url}}/pets?limit=10&species={{species}}");
    assert_eq!(list.directives.auth.as_deref(), Some("bearer {{token}}"));
    assert_eq!(list.headers.len(), 1);
    assert_eq!(list.header("accept"), Some("application/json"));
    let list_md = read(&root, "Pets/List pets.md");
    assert!(list_md.contains("Insomnia script not imported"));
    assert_eq!(page::parse(&list_md).front_matter["species"], "dog");

    let create = only(&pages, "Pets/Create pet.md");
    assert_eq!(create.method, "POST");
    assert_eq!(create.directives.auth.as_deref(), Some("bearer {{token}}"));
    assert!(create.directives.no_redirect);
    let body = create.body.as_deref().unwrap();
    assert!(body.contains("\"species\": \"{{species}}\""));
    assert!(body.contains("\"tag\": \"{{$uuid}}\""));

    let upload = only(&pages, "Pets/Upload photo.md");
    assert_eq!(upload.url, "{{base_url}}/pets/7/photo");
    assert_eq!(
        upload.directives.auth.as_deref(),
        Some("apikey X-Upload-Key {{upload-key}}")
    );
    assert!(upload.header("content-type").unwrap().contains("boundary="));
    assert!(upload.body.as_deref().unwrap().contains("Rex at the beach"));

    let token = only(&pages, "Get token.md");
    assert_eq!(
        token.directives.auth.as_deref(),
        Some("basic {{client_id}} {{client_secret}}")
    );
    assert_eq!(
        token.body.as_deref(),
        Some("grant_type=client_credentials&scope=pets%3Aread%20pets%3Awrite")
    );

    let gql = only(&pages, "Pet by id (GraphQL).md");
    assert_eq!(gql.kind, Kind::GraphQl);
    assert_eq!(gql.directives.auth, None);
    assert!(
        gql.body
            .as_deref()
            .unwrap()
            .starts_with("query Pet($id: ID!) {\n  pet(id: $id)")
    );
    assert!(gql.body.as_deref().unwrap().ends_with("\"id\": \"7\"\n}"));

    let me = only(&pages, "Who am I.md");
    assert!(me.header("authorization").unwrap().contains("{% response"));
    assert!(read(&root, "Who am I.md").contains("template tags reqowl can't evaluate"));

    let ws_req = only(&pages, "Live feed.md");
    assert_eq!(ws_req.kind, Kind::WebSocket);
    assert_eq!(ws_req.url, "wss://pets.example.com/feed");
}

#[test]
fn insomnia_v5() {
    assert_eq!(
        import::detect(&fixture("insomnia_v5.yaml")).unwrap(),
        Format::InsomniaV5
    );
    let (_tmp, root) = import_fixture("insomnia_v5.yaml");
    let pages = load_pages(&root);

    let ws = Workspace::open(&root).unwrap();
    assert_eq!(ws.config.name.as_deref(), Some("Insomnia Airlines"));
    assert_eq!(ws.defaults().unwrap()["test-value"], "collection");
    assert_eq!(ws.env_names().unwrap(), ["Personal", "Staging"]);
    assert_eq!(ws.load_env("Staging").unwrap().vars["api_key"], "staging-key");
    assert_eq!(ws.load_env("Personal").unwrap().secrets["api_key"], "my-own-key");

    assert_eq!(order(&root, ""), ["About", "airports", "grandparent", "Create booking"]);
    assert_eq!(order(&root, "airports"), ["List airports", "Update an airport"]);

    // Folder apikey auth; collection-level header; disabled query parameter dropped.
    let list = only(&pages, "airports/List airports.md");
    assert_eq!(list.url, "{{base_url}}/airports?limit=2");
    assert_eq!(list.directives.auth.as_deref(), Some("apikey X-Api-Key {{api_key}}"));
    assert_eq!(list.header("x-client"), Some("reqowl-test"));
    assert!(read(&root, "airports/List airports.md").contains("```js\ninsomnia.test("));

    let update = only(&pages, "airports/Update an airport.md");
    assert_eq!(
        (update.method.as_str(), update.url.as_str()),
        ("PUT", "{{base_url}}/airports/YUL")
    );
    assert!(update.directives.no_redirect);
    assert!(update.body.as_deref().unwrap().contains("Montréal–Trudeau"));
    assert!(read(&root, "airports/Update an airport.md").starts_with("# Update an airport.\n"));

    // Deep nesting, bracket variables, folder environment, basic auth with empty password.
    let child = only(&pages, "grandparent/parent/Child Request.md");
    assert_eq!(child.header("x-test-value"), Some("{{test-value}}"));
    assert_eq!(child.directives.auth.as_deref(), Some("basic {{test-value}}:"));
    let fm = page::parse(&read(&root, "grandparent/parent/Child Request.md")).front_matter;
    assert_eq!(fm["test-value"], "grandparent");

    // Collection-level bearer inherited at the root.
    let booking = only(&pages, "Create booking.md");
    assert_eq!(booking.directives.auth.as_deref(), Some("bearer {{token}}"));
    assert!(read(&root, "Create booking.md").contains("pre-request script"));

    let mut auth = child.clone();
    auth.directives.auth = Some("basic user:".into());
    auth.apply_auth().unwrap();
    assert_eq!(auth.header("authorization"), Some("Basic dXNlcjo="));
}

#[test]
fn curl_command() {
    let cmd = "curl -X POST 'https://api.example.com/v1/items?draft=true' \\\n  -H 'Content-Type: application/json' \\\n  -u alice:pw \\\n  -k --max-time 10 \\\n  -d '{\"name\": \"lamp\"}'";
    assert_eq!(import::detect(cmd).unwrap(), Format::Curl);
    let ws = import::detect_and_parse(cmd).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    import::write_workspace(&ws, tmp.path()).unwrap();
    let pages = load_pages(tmp.path());
    let r = only(&pages, "Imported request.md");
    assert_eq!(
        (r.method.as_str(), r.url.as_str()),
        ("POST", "https://api.example.com/v1/items?draft=true")
    );
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_eq!(r.directives.auth.as_deref(), Some("basic alice pw"));
    assert!(r.directives.insecure);
    assert_eq!(r.directives.timeout, Some(Duration::from_secs(10)));
    assert_eq!(r.body.as_deref(), Some("{\"name\": \"lamp\"}"));

    // Back out as curl and in again.
    let again = import::curl::parse(&import::to_curl(r)).unwrap();
    assert_eq!(&again, r);
}

#[test]
fn refuses_existing_workspace_and_keeps_existing_files() {
    let ws = import::detect_and_parse("curl https://h/x").unwrap();
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("Imported request.md"), "mine").unwrap();
    fs::write(tmp.path().join(".gitignore"), "target/").unwrap();
    import::write_workspace(&ws, tmp.path()).unwrap();
    assert_eq!(
        fs::read_to_string(tmp.path().join("Imported request.md")).unwrap(),
        "mine"
    );
    assert!(tmp.path().join("Imported request (2).md").is_file());
    assert_eq!(
        fs::read_to_string(tmp.path().join(".gitignore")).unwrap(),
        "target/\n.reqowl/\nenv/*.local.toml\n"
    );
    let err = import::write_workspace(&ws, tmp.path()).unwrap_err();
    assert!(matches!(err, Error::Import(_)), "{err}");
}

#[test]
fn awkward_names_and_methods() {
    let col = r#"{
        "info": {"name": "Odd", "schema": "https://schema.getpostman.com/json/collection/v2.0.0/collection.json"},
        "item": [
            {"name": "env", "item": [{"name": "a/b", "request": {"method": "GET", "url": "http://h/1"}}]},
            {"name": "a:b", "request": {"method": "PROPFIND", "url": "http://h/dav"}},
            {"name": "A?B", "request": {"method": "GET", "url": "", "header": [{"key": "Bad Name", "value": "x"}, {"key": "Ok", "value": "multi\nline"}]}},
            {"name": "Fence", "request": {"method": "POST", "url": "http://h", "body": {"mode": "raw", "raw": "```\ncode\n```\r\n\r\n"}}}
        ],
        "auth": {"type": "basic", "basic": {"username": "u", "password": ""}}
    }"#;
    let ws = import::detect_and_parse(col).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    import::write_workspace(&ws, tmp.path()).unwrap();
    let root = tmp.path();
    // A folder called `env` must not land in the environments directory.
    assert_eq!(order(root, ""), ["env (2)", "ab", "AB (2)", "Fence"]);
    assert!(root.join("env (2)/ab.md").is_file());
    let pages = load_pages(root);
    assert!(pages["ab.md"].is_empty(), "unsupported method is not a runnable block");
    assert!(read(root, "ab.md").contains("```text\n"));
    let ab = only(&pages, "AB (2).md");
    assert_eq!(ab.url, "{{url}}");
    assert_eq!(ab.headers.len(), 1);
    assert_eq!(ab.header("ok"), Some("multi line"));
    // v2.0 auth object form.
    assert_eq!(ab.directives.auth.as_deref(), Some("basic u:"));
    let fence = only(&pages, "Fence.md");
    assert_eq!(fence.body.as_deref(), Some("```\ncode\n```"));
}
