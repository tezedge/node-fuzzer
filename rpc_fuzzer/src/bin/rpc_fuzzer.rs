use urlencoding::encode;
use clap::{App, Arg};
use rand::{ rngs::SmallRng, SeedableRng, Rng };
use serde_json::Value;
use hyper::{Body, Method, Client, Request};
use futures::future::join_all;

#[derive(Clone)]
enum ParamSchema {
    String,
    Integer
}

#[derive(Clone)]
struct Param {
    name: String,
    schema: ParamSchema,
    required: bool
}

#[derive(Clone)]
struct PathMethod {
    method: Method,
    path_params: Vec<Param>,
    query_params: Vec<Param>
}

#[derive(Clone)]
struct Path {
    name: String,
    methods: Vec<PathMethod>
}

fn random_len(rng: &mut SmallRng, min_size: usize, max_size: usize) -> usize {
    if rng.gen_bool(0.3) {
        return 15
    }

    if rng.gen_bool(0.3) {
        return 51
    }

    rng.gen_range(min_size..max_size)
}

fn random_number(rng: &mut SmallRng, size: usize) -> String {
    let prefix = ["", "-"];
    const CHARSET: &[u8] = b"0123456789";
    let start = rng.gen_range(0..2);
    let string: String = (start..size)
    .map(|_|{CHARSET[rng.gen_range(0..CHARSET.len())] as char}).collect();
    prefix[start].to_string() + &string
}

fn random_string(rng: &mut SmallRng, size: usize) -> String {
    let mut string: String = (0..size).map(|_|{rng.gen::<char>()}).collect();
        
    for i in (0..size).rev() {
        if string.is_char_boundary(i) {
            string.truncate(i);
            break;
        }
    }

    string
}

fn random_arg(rng: &mut SmallRng) -> String {
    let hardcoded_strings = [
        "main",
        "test",
        "NetXdQprcVkpaWU",
        "BLockGenesisGenesisGenesisGenesisGenesisf79b5d1CoW2"
    ];

    let size = random_len(rng, 1, 100);

    if rng.gen_bool(0.1) {
        let hc_str = hardcoded_strings[rng.gen_range(0..hardcoded_strings.len())];
        return hc_str.to_string()
    }

    if rng.gen_bool(0.2) {
        return encode(random_string(rng, size).as_str()).to_string()
    }

    if rng.gen_bool(0.2) {
        return random_number(rng, size)
    }

    const CHARSET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let string: String = (0..size)
    .map(|_|{CHARSET[rng.gen_range(0..CHARSET.len())] as char}).collect();
    string
}

fn random_json_list(rng: &mut SmallRng, size: usize) -> String {
    let mut remaining = size;
    let mut ret = "[".to_string();

    loop {
        if remaining == 0 {
            break
        }

        let element_size = rng.gen_range(0..remaining + 1);
        remaining -= element_size;
        ret.push_str(random_json(rng, element_size).as_str());

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
            break
        }

        let key_size = rng.gen_range(0..remaining + 1);
        remaining -= key_size;
        ret.push_str(format!("\"{}\"", random_string(rng, key_size)).as_str());

        if remaining == 0 {
            ret.push_str(": null");
            break
        }

        let element_size = rng.gen_range(0..remaining + 1);
        remaining -= element_size;
        ret.push_str(format!(": {}", random_json(rng, element_size)).as_str());

        if remaining != 0 {
            ret.push_str(",");
        }
    }
    ret.push_str("}");
    ret
}

fn random_json(rng: &mut SmallRng, size: usize) -> String {
    match rng.gen_range(0..10) {
        0 => "".to_string(),
        1 => "true".to_string(),
        2 => "false".to_string(),
        3 => "null".to_string(),
        4 => format!("\"{}\"", random_number(rng, size)),
        5 => format!("\"{}\"", random_string(rng, size)),
        6 => random_json_list(rng, size),
        _ => random_json_dict(rng, size)
    }
}
/*
    TODO: 
      - use query responses to find valid hashes/ids and use them as input
      - improve random json structure aware from openapi schema
*/

async fn worker(target: String, paths: Vec<Path>, seed: u64) {
    let mut rng = SmallRng::seed_from_u64(seed);
    let client = Client::new();

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
                random_arg(&mut rng).as_str()
            );
        }

        // workaround for missing parts of openapi
        name = name.replace( 
            "{chain_id}",
            random_arg(&mut rng).as_str()
        );
        name = name.replace( 
            "{block_id}",
            random_arg(&mut rng).as_str()
        );

        let mut param_prefix = "?".to_string();

        for param in &method.query_params {
            //param_name.push_str(&param.name);
            if rng.gen_bool(0.5) {
                name.push_str(&param_prefix);
                name.push_str(&param.name);

                if rng.gen_bool(0.5) {
                    name.push_str("=");
                    name.push_str(random_arg(&mut rng).as_str());
                }

                param_prefix = "&".to_string();
            }
        }
        
        //eprintln!("{}", name);
        println!("http://{}{}", target, name);
        let size = random_len(&mut rng, 0x5000, 0x10000);
        let body = Body::from(random_json(&mut rng, size));

        let req = Request::builder()
        .method(&method.method)
        .uri(format!("http://{}{}", target, name))
        .body(body).unwrap();
        
        let resp = client.request(req).await;

        if resp.is_err() {
            let err = resp.unwrap_err();
            //err.is_connect()
            eprintln!("{}", err);
            return
        }

        //println!("Response: {}", resp.unwrap().status());
    }
}

fn parse_parameter(param: &Value) -> Param {
    // assume schemas are string or integer
    let schema = match param["schema"].as_object().unwrap()["type"].as_str().unwrap() {
        "string" => ParamSchema::String,
        "integer" => ParamSchema::Integer,
        _ => panic!("Unknown parameter schema")    
    };

    let required = match param["required"].as_str() {
        Some("true") => true,
        _ => false
    };

    Param {
        name: param["name"].as_str().unwrap().to_string(),
        schema: schema,
        required: required
    }
}

fn parse_method(meth: &String, value: &Value) -> PathMethod {
    let method = match meth.as_str() {
        "get" => Method::GET,
        "patch" => Method::PATCH,
        "post" => Method::POST,
        "delete" => Method::DELETE,
        _ => panic!("Unknown method")
    };

    let meth_info = value.as_object().unwrap();
    //eprintln!("{}", meth);
    /* TODO
    if meth_info.contains_key("requestBody") {
    }
    */
    
    let mut path_params = Vec::new();
    let mut query_params = Vec::new();

    if meth_info.contains_key("parameters") {
        for param in meth_info["parameters"].as_array().unwrap() {
            //eprintln!("{}", param["name"]);
            match param["in"].as_str().unwrap() {
                "path" => path_params.push(parse_parameter(param)),
                "query" => query_params.push(parse_parameter(param)),
                _ => panic!("Unknown parameter")
            }    
        }     
    }

    PathMethod { method, path_params, query_params }
}

fn parse_path(path: &String, value: &Value) -> Path {
    Path {
        name: path.clone(),
        methods: value.as_object().unwrap().iter()
        .map(|(meth, value)| {parse_method(meth, value)}).collect()
    }
}


#[tokio::main]
async fn main() {
    let matches = App::new("rpc_fuzzer")
        .arg(
            Arg::with_name("node")
                .short("n")
                .long("node")
                .default_value("localhost")
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
                .default_value("4")
                .help("number of fuzzer threads"),
        )
        .get_matches();

    let target = matches.value_of("node").unwrap();
    let seed = matches.value_of("seed").unwrap().parse::<u64>().unwrap();
    let threads = matches.value_of("threads").unwrap().parse::<u64>().unwrap();
    let _max_fd = fdlimit::raise_fd_limit().unwrap();
    //eprintln!("current fd limit {}", _max_fd);
    let openapi = std::fs::read_to_string("tezedge-openapi.json").unwrap();
    let schema: Value = serde_json::from_str(&openapi).unwrap();
    let paths: Vec<Path> = schema.as_object().unwrap()["paths"].as_object()
    .unwrap().iter()
    .map(|(path, value)| {parse_path(path, value)}).collect();
 
    let mut tasks = Vec::new();

    for n in 0..threads {
        let _target = target.to_string();
        let _paths = paths.clone();

        tasks.push(
            tokio::spawn(async move { worker(_target, _paths, seed * n).await })
        );
    }

    join_all(tasks).await;
}
