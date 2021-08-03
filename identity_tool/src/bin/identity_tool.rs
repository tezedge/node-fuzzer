use clap::{App, Arg};
//use tokio::runtime::Handle;
use futures::future::join_all;
use tezos_identity::Identity;
use crypto::proof_of_work::ProofOfWork;


#[tokio::main]
async fn main() {
    let matches = App::new("identity_tool")
        .arg(
            Arg::with_name("number")
                .short("n")
                .long("number")
                .takes_value(true)
                .help("number of identities to generate"),
        )
        .get_matches();

    let num = match matches.value_of("number") {
        Some(num) => String::from(num).parse::<u64>().unwrap(),
        None => 1,
    };

    let mut tasks = Vec::new();

    println!("[");
    let num = num - 1;

    for _ in 0..16 {
        tasks.push(tokio::spawn(async move {
            for _ in 0..num/16 {
                let id = Identity::generate(26.0).unwrap();
                println!("{}, ", id.as_json().unwrap());
            }
        }));
    }

    for _ in 0..num % 16 {
        let id = Identity::generate(26.0).unwrap();
        println!("{}, ", id.as_json().unwrap());
    }

    join_all(tasks).await;

    let id = Identity::generate(26.0).unwrap();
    println!("{}", id.as_json().unwrap());
    println!("]");
}
