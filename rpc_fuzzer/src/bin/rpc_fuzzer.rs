// Copyright (c) SimpleStaking, Viable Systems and Tezedge Contributors
// SPDX-License-Identifier: MIT

use clap::{App, Arg};
use futures::{future::join_all, Future};
use hyper::{Body, Client, Method, Request, Response};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use serde_json::Value;
use serde_json_schema::Schema;
use std::{
    borrow::Borrow, collections::HashSet, convert::TryFrom, sync::Arc, time::Duration,
};
use tokio::time::{Timeout, sleep, timeout};

#[derive(Clone)]
enum ParamSchema {
    String,
    Integer,
}

#[derive(Clone)]
struct Param {
    name: String,
    schema: ParamSchema,
    required: bool,
}

#[derive(Clone)]
struct PathMethod {
    method: Method,
    path_params: Vec<Param>,
    query_params: Vec<Param>,
    body: Arc<Option<Schema>>,
}

#[derive(Clone)]
struct Path {
    name: String,
    methods: Vec<PathMethod>,
}

fn random_len(rng: &mut SmallRng, min_size: usize, max_size: usize) -> usize {
    rng.gen_range(min_size..max_size)
}

fn random_number(rng: &mut SmallRng, size: usize) -> String {
    let prefix = if rng.gen_bool(0.9) { "" } else { "-" };
    const CHARSET: &[u8] = b"0123456789";
    let string: String = (prefix.len()..size)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect();
    prefix.to_string() + &string
}

fn random_string(rng: &mut SmallRng, size: usize) -> String {
    if rng.gen_bool(0.5) {
        const CHARSET: &[u8] = b"0123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let string: String = (0..size)
            .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
            .collect();
        return string;
    }

    let mut string: String = (0..size).map(|_| rng.gen::<char>()).collect();

    for i in (0..size).rev() {
        if string.is_char_boundary(i) {
            string.truncate(i);
            break;
        }
    }

    string
}

fn random_arg(
    rng: &mut SmallRng,
    chains: Option<&HashSet<String>>,
    branches: Option<&HashSet<String>>,
    blocks: Option<&HashSet<String>>,
) -> String {
    let aliases = ["main", "test", "head"];
    let size = random_len(rng, 1, 30);

    if let Some(chains) = chains {
        for chain in chains {
            if rng.gen_bool(0.1) {
                return chain.to_string();
            }
        }
    }

    if let Some(branches) = branches {
        for branch in branches {
            if rng.gen_bool(0.1) {
                return branch.to_string();
            }
        }
    }

    if let Some(blocks) = blocks {
        for block in blocks {
            if rng.gen_bool(0.1) {
                return block.to_string();
            }
        }
    }

    if rng.gen_bool(0.5) {
        let hc_str = aliases[rng.gen_range(0..aliases.len())];

        if hc_str.contains("head") && rng.gen_bool(0.5) {
            return hc_str.to_string() + &"~".to_string() + &random_number(rng, size);
        }

        return hc_str.to_string();
    }

    random_number(rng, size)
}

fn random_json_list(rng: &mut SmallRng, prop: Option<&PropertyInstance>, size: usize) -> String {
    let mut remaining = size;
    let mut ret = "[".to_string();

    loop {
        if remaining == 0 {
            break;
        }

        let element_size = rng.gen_range(0..remaining + 1);
        remaining -= element_size;
        ret.push_str(random_json(rng, prop, element_size).as_str());

        if remaining != 0 {
            ret.push_str(",");
        }
    }
    ret.push_str("]");
    ret
}

fn random_json_dict(rng: &mut SmallRng, size: usize) -> String {
    let mut remaining = size;
    let mut ret = "{".to_string();

    loop {
        if remaining == 0 {
            break;
        }

        let key_size = rng.gen_range(0..remaining + 1);
        remaining -= key_size;
        ret.push_str(format!("\"{}\"", random_string(rng, key_size)).as_str());

        if remaining == 0 {
            ret.push_str(": null");
            break;
        }

        let element_size = rng.gen_range(0..remaining + 1);
        remaining -= element_size;
        ret.push_str(format!(": {}", random_json(rng, None, element_size)).as_str());

        if remaining != 0 {
            ret.push_str(",");
        }
    }
    ret.push_str("}");
    ret
}

use serde_json_schema::property::{Property, PropertyInstance};

/*
    Generates random JSON (used to generate POST/PATCH request's body)
    If available it can use information from requestBody's schema.

    TODO:
    The serde-json-schema crate used to parse the requestBody schema
    is limited/incomplete and doesn't allow us to extract all the
    information needed to generate proper JSON. It will be replaced by
    "jtd" and follow a similar generation approach to:
    https://github.com/jsontypedef/json-typedef-fuzz/blob/master/src/lib.rs

*/

fn random_json(rng: &mut SmallRng, prop: Option<&PropertyInstance>, size: usize) -> String {
    let code = match prop {
        Some(p) => {
            match p {
                PropertyInstance::Null => "null".to_string(),
                PropertyInstance::Boolean(_) => {
                    if rng.gen_bool(0.5) {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                PropertyInstance::Integer { .. } => {
                    format!("\"{}\"", random_number(rng, size))
                }
                PropertyInstance::Number { ..  } => {
                    format!("\"{}\"", random_number(rng, size))
                }
                PropertyInstance::Array { items } => random_json_list(rng, Some(&*items), size),
                PropertyInstance::Object {
                    properties,
                    ..
                } => {
                    let mut ret = "{\"".to_string();

                    for (name, prop) in properties {
                        ret.push_str(name);
                        match prop {
                            Property::Value(p) => {
                                ret.push_str(
                                    format!("\": {},", random_json(rng, Some(p), size)).as_str(),
                                );
                            }
                            Property::Ref(refp) => {
                                ret.push_str(refp.reference.as_str());
                                ret.push_str(",");
                            }
                        }
                    }

                    if ret.len() > 1 {
                        ret.pop();
                    }
                    ret.push_str("}");
                    ret
                }
                PropertyInstance::String => format!("\"{}\"", random_string(rng, size)),
            }
        }
        None => match rng.gen_range(0..10) {
            0 => "true".to_string(),
            1 => "false".to_string(),
            2 => "null".to_string(),
            3 => format!("\"{}\"", random_number(rng, size)),
            4 => format!("\"{}\"", random_string(rng, size)),
            5 => random_json_list(rng, None, size),
            _ => random_json_dict(rng, size),
        },
    };
    code
}

async fn sleep_ms(duration: u64) {
    if duration > 0 {
        sleep(Duration::from_millis(duration)).await;
    }
}

async fn worker(target: String, paths: Vec<Path>, seed: u64) {
    let mut rng = SmallRng::seed_from_u64(seed);
    let client = Client::new();
    let mut chains = HashSet::new();
    let mut branches = HashSet::new();
    let mut blocks = HashSet::new();

    loop {
        let path = &paths[rng.gen_range(0..paths.len())];
        let method = &path.methods[rng.gen_range(0..path.methods.len())];
        let mut name = path.name.clone();

        for param in &method.path_params {
            let mut param_name = "{".to_string();
            param_name.push_str(&param.name);
            param_name.push_str("}");
            name = name.replace(
                param_name.as_str(),
                random_arg(&mut rng, Some(&chains), Some(&branches), Some(&blocks)).as_str(),
            );
        }

        // workaround for missing parts of openapi
        name = name.replace(
            "{chain_id}",
            random_arg(&mut rng, Some(&chains), None, None).as_str(),
        );
        name = name.replace(
            "{block_id}",
            random_arg(&mut rng, None, None, Some(&blocks)).as_str(),
        );

        let mut param_prefix = "?".to_string();

        for param in &method.query_params {
            if rng.gen_bool(0.5) {
                name.push_str(&param_prefix);
                name.push_str(&param.name);

                if rng.gen_bool(0.5) {
                    name.push_str("=");
                    name.push_str(
                        random_arg(&mut rng, Some(&chains), Some(&branches), Some(&blocks))
                            .as_str(),
                    );
                }

                param_prefix = "&".to_string();
            }
        }

        let size = random_len(&mut rng, 0x500, 0x5000);
        let body = if method.method == Method::PATCH || method.method == Method::POST {
            match method.body.borrow() {
                Some(schema) => Body::from(random_json(&mut rng, schema.specification(), size)),
                None => Body::from(random_json(&mut rng, None, size)),
            }
        } else {
            Body::empty()
        };

        let req = Request::builder()
            .method(&method.method)
            .uri(format!("http://{}{}", target, name))
            .body(body)
            .unwrap();

        match with_timeout(5, client.request(req)).await {
            Ok(resp) => match resp {
                Err(err) => {
                    eprintln!("!!! {}", err);
                    sleep_ms(5000).await
                }
                Ok(mut response) => match recv_body(&mut response).await {
                    Some(body) => {
                        collect_reply_data(&body, &mut chains, &mut branches, &mut blocks);
                        println!(
                            "[{}] http://{}{} | {}",
                            response.status(),
                            target,
                            name,
                            body
                        );
                        println!(
                            "[INFO] chains {:?}, branches {:?}, blocks {:?}",
                            &chains, &branches, &blocks
                        );
                    }
                    None => {
                        println!(
                            "[{}] http://{}{} | No body",
                            response.status(),
                            target,
                            name
                        );
                    }
                },
            },
            _ => println!("[TIMEOUT] http://{}{}", target, name),
        }
    }
}

async fn recv_body<T>(response: &mut Response<T>) -> Option<Value>
where
    T: hyper::body::HttpBody + std::marker::Unpin,
{
    let body_fut = hyper::body::to_bytes(response.body_mut());

    match with_timeout(1, body_fut).await {
        Ok(body) => {
            let body = match body {
                Ok(bytes) => bytes.to_vec(),
                _ => panic!("error getting body bytes"),
            };

            if body.len() > 0x1000 {
                println!("[WARN] Skipping large Body");
                return None;
            } else {
                match serde_json::from_slice(body.as_slice()) {
                    Ok(body) => body,
                    Err(e) => {
                        println!("[WARN] {:?}: {}", e, String::from_utf8_lossy(&body));
                        None
                    }
                }
            }
        }
        _ => {
            println!("[WARN] Body timeout (Stream?)");
            None
        }
    }
}

fn with_timeout<T>(seconds: u64, future: T) -> Timeout<T>
where
    T: Future,
{
    timeout::<T>(Duration::from_millis(1000 * seconds), future)
}

fn collect_reply_data(
    value: &Value,
    chains: &mut HashSet<String>,
    branches: &mut HashSet<String>,
    blocks: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_reply_data(value, chains, branches, blocks);
            }
        }
        serde_json::Value::Object(dict) => {
            for (k, value) in dict {
                // TODO: extract more data (ej contracts)
                match k.as_str() {
                    "branch" => {
                        branches.insert(value.as_str().unwrap().to_string());
                    }
                    "chain_id" => {
                        chains.insert(value.as_str().unwrap().to_string());
                    }
                    "block" => {
                        blocks.insert(value.as_str().unwrap().to_string());
                    }
                    "block_hash" => {
                        blocks.insert(value.as_str().unwrap().to_string());
                    }
                    "predecessor" => {
                        blocks.insert(value.as_str().unwrap().to_string());
                    }
                    _ => collect_reply_data(value, chains, branches, blocks),
                };
            }
        }
        _ => (),
    }
}

fn parse_parameter(param: &Value) -> Param {
    /*
        TODO: use JSON schema parser (jtd)
        right now we assume schemas are string or integer
    */
    let schema = match param["schema"].as_object().unwrap()["type"]
        .as_str()
        .unwrap()
    {
        "string" => ParamSchema::String,
        "integer" => ParamSchema::Integer,
        _ => panic!("Unknown parameter schema"),
    };

    let required = match param["required"].as_str() {
        Some("true") => true,
        _ => false,
    };

    Param {
        name: param["name"].as_str().unwrap().to_string(),
        schema: schema,
        required: required,
    }
}

fn parse_method(meth: &String, value: &Value) -> PathMethod {
    let method = match meth.as_str() {
        "get" => Method::GET,
        "patch" => Method::PATCH,
        "post" => Method::POST,
        "put" => Method::PUT,
        "delete" => Method::DELETE,
        m => panic!("Unknown method {}", m),
    };

    let meth_info = value.as_object().unwrap();
    let mut path_params = Vec::new();
    let mut query_params = Vec::new();

    if meth_info.contains_key("parameters") {
        for param in meth_info["parameters"].as_array().unwrap() {
            match param["in"].as_str().unwrap() {
                "path" => path_params.push(parse_parameter(param)),
                "query" => query_params.push(parse_parameter(param)),
                _ => panic!("Unknown parameter"),
            }
        }
    }

    let mut body = None;

    /*
        Parse requestBody, this is important for POST/PATCH RPCs like injection endpoints
    */
    if meth_info.contains_key("requestBody") {
        if let Some(req_body) = meth_info["requestBody"].as_object() {
            if let Some(content) = req_body["content"].as_object() {
                if let Some(app) = content["application/json"].as_object() {
                    if let Ok(schema) = Schema::try_from(app["schema"].to_string()) {
                        println!("new schema {}", app["schema"].to_string());
                        body = Some(schema);
                    } else {
                        println!("error parsing schema {}", app["schema"].to_string());
                    }
                }
            }
        }
    }

    PathMethod {
        method,
        path_params,
        query_params,
        body: Arc::new(body),
    }
}

fn parse_path(path: &String, value: &Value) -> Path {
    Path {
        name: path.clone(),
        methods: value
            .as_object()
            .unwrap()
            .iter()
            .map(|(meth, value)| parse_method(meth, value))
            .collect(),
    }
}

#[tokio::main]
async fn main() {
    let matches = App::new("rpc_fuzzer")
        .arg(
            Arg::with_name("node")
                .short("n")
                .long("node")
                .default_value("172.18.0.101:18732")
                .help("IP or hostname of target node"),
        )
        .arg(
            Arg::with_name("seed")
                .short("s")
                .long("seed")
                .default_value("1234567890")
                .help("PRNG seed"),
        )
        .arg(
            Arg::with_name("threads")
                .short("t")
                .long("threads")
                .default_value("20")
                .help("number of fuzzer threads"),
        )
        .arg(
            Arg::with_name("shell_schemas")
                .short("S")
                .long("shell_schemas")
                .default_value("./rpc_shell_schemas/")
                .help("Path containing shell RPCs (OpenAPI) JSON schemas"),
        )
        .arg(
            Arg::with_name("protocol_schemas")
                .short("P")
                .long("protocol_schemas")
                .default_value("./rpc_protocol_schemas/")
                .help("Path containing protocol RPCs (OpenAPI) JSON schemas"),
        )
        .get_matches();

    let target = matches.value_of("node").unwrap();
    let seed = matches.value_of("seed").unwrap().parse::<u64>().unwrap();
    let threads = matches.value_of("threads").unwrap().parse::<u64>().unwrap();
    let _max_fd = fdlimit::raise_fd_limit().unwrap();

    let mut paths: Vec<Path> = Vec::new();

    for file in std::fs::read_dir(matches.value_of("shell_schemas").unwrap()).unwrap() {
        let openapi = std::fs::read_to_string(file.unwrap().path()).unwrap();
        let schema: Value = serde_json::from_str(&openapi).unwrap();
        for (path, value) in schema.as_object().unwrap()["paths"].as_object().unwrap() {
            paths.push(parse_path(path, value));
        }
    }

    for file in std::fs::read_dir(matches.value_of("protocol_schemas").unwrap()).unwrap() {
        let openapi = std::fs::read_to_string(file.unwrap().path()).unwrap();
        let schema: Value = serde_json::from_str(&openapi).unwrap();
        for (path, value) in schema.as_object().unwrap()["paths"].as_object().unwrap() {
            let prefix = "/chains/{chain_id}/blocks/{block_id}".to_owned();
            paths.push(parse_path(&(prefix + path), value));
        }
    }

    let mut tasks = Vec::new();

    for n in 0..threads {
        let _target = target.to_string();
        let _paths = paths.clone();

        tasks.push(tokio::spawn(async move {
            worker(_target, _paths, seed * n).await
        }));
    }

    join_all(tasks).await;
}
