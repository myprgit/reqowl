//! A whole page against a mock server: environments, chaining, auth, GraphQL.

use std::fs;

use reqowl_core::error::Error;
use reqowl_core::page;
use reqowl_core::session::{Session, find_block};
use wiremock::matchers::{body_json, body_string, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PAGE: &str = r#"---
user: alice
---
# Login flow

First get a token:

```http
# @name login
POST {{base}}/login
Content-Type: application/json

{"user": "{{user}}", "password": "{{password}}"}
```

Then use it:

```http
# @auth bearer {{login.response.body.$.token}}
GET {{base}}/orders
  ?limit=2
```

```http
GRAPHQL {{base}}/graphql

query Me($id: ID!) { me(id: $id) { name } }

{"id": "{{login.response.body.$.id}}"}
```
"#;

async fn setup(server: &MockServer) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("reqowl.toml"), "default_env = \"dev\"\n").unwrap();
    fs::create_dir_all(root.join("env")).unwrap();
    fs::create_dir_all(root.join("Auth")).unwrap();
    fs::write(root.join("env/dev.toml"), format!("base = \"{}\"\n", server.uri())).unwrap();
    fs::write(root.join("env/dev.local.toml"), "password = \"hunter2\"\n").unwrap();
    fs::write(root.join("Auth/Login.md"), PAGE).unwrap();
    dir
}

#[tokio::test]
async fn chained_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login"))
        .and(body_string(r#"{"user": "alice", "password": "hunter2"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "tok-1", "id": 42})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/orders"))
        .and(query_param("limit", "2"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id": 1}])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_json(serde_json::json!({
            "query": "query Me($id: ID!) { me(id: $id) { name } }",
            "variables": {"id": "42"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":{"me":{"name":"Alice"}}}"#))
        .mount(&server)
        .await;

    let dir = setup(&server).await;
    let page_path = dir.path().join("Auth/Login.md");
    let page = page::parse(&fs::read_to_string(&page_path).unwrap());
    assert_eq!(page.blocks.len(), 3);

    let mut s = Session::open(&page_path, &page, None).unwrap();
    assert_eq!(s.env.as_ref().unwrap().name, "dev");
    assert_eq!(s.ctx.secret_values(), [("password", "hunter2")]);

    // Running the second block first fails: `login` has no response yet.
    let err = s.run(&page.blocks[1]).await.err().unwrap();
    assert!(matches!(err, Error::Variable(_)), "{err}");

    for block in &page.blocks {
        let out = s
            .run(block)
            .await
            .unwrap_or_else(|e| panic!("block {}: {e}", block.index + 1));
        assert_eq!(
            out.response.status,
            200,
            "block {}: {}",
            block.index + 1,
            out.response.body_text()
        );
    }
    assert!(find_block(&page, "login").is_some());
    assert_eq!(find_block(&page, "3").unwrap().index, 2);
    assert!(find_block(&page, "4").is_none());
}

#[tokio::test]
async fn redirects_and_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/old"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/new"))
        .mount(&server)
        .await;
    Mock::given(path("/new"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let dir = setup(&server).await;
    let page_path = dir.path().join("Auth/Redirect.md");
    let text = "```http\nGET {{base}}/old\n```\n\n```http\n# @no-redirect\nGET {{base}}/old\n```\n\n\
```http\nGET {{base}}/{{nope}}/{{nada}}\n```\n\n```http\n# @timeout 2s\nGET http://127.0.0.1:1/\n```\n";
    fs::write(&page_path, text).unwrap();
    let page = page::parse(text);
    let mut s = Session::open(&page_path, &page, Some("dev")).unwrap();

    let out = s.run(&page.blocks[0]).await.unwrap();
    assert_eq!(out.response.status, 204);
    assert!(out.response.url.ends_with("/new"));
    assert_eq!(s.run(&page.blocks[1]).await.unwrap().response.status, 302);

    match s.run(&page.blocks[2]).await {
        Err(Error::UndefinedVariables(v)) => assert_eq!(v, ["nope", "nada"]),
        other => panic!("{:?}", other.err()),
    }
    let err = s.run(&page.blocks[3]).await.err().unwrap();
    assert!(matches!(err, Error::Http(_)), "{err}");

    assert!(matches!(
        Session::open(&page_path, &page, Some("prod")),
        Err(Error::UnknownEnv(_))
    ));
}
