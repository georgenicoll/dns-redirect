use std::collections::HashMap;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use hickory_proto::op::{Header, ResponseCode};
use hickory_proto::rr::rdata::CNAME;
use hickory_proto::rr::{LowerName, Name, RData, Record, RecordType};
use hickory_server::authority::MessageResponseBuilder;
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo, ServerFuture};
use serde::{Deserialize, Deserializer};
use tokio::net::UdpSocket;


fn deserialize_regex<'de, D>(deserializer: D) -> Result<regex::Regex, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    regex::Regex::new(&s).map_err(serde::de::Error::custom)
}

#[derive(Clone, Deserialize)]
struct Replacement {
    #[serde(deserialize_with = "deserialize_regex")]
    from: regex::Regex,
    to: String,
}


#[derive(Deserialize)]
struct Config {
    bind_address: String,
    replacements: Vec<Replacement>,
}

impl Config {
    fn load_from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| e.into())
    }
}


#[derive(Clone)]
struct DomainConversionHandler {
    replacements: Vec<Replacement>,
}

impl DomainConversionHandler {
    const fn new(replacements: Vec<Replacement>) -> Self {
        Self { replacements }
    }

    fn find_replacement(&self, name: &LowerName) -> Option<String> {
        self.replacements.iter().find_map(|replacement| {
            replacement.from.captures(&name.to_utf8()).map(|caps| {
                let vars: HashMap<String, String> = caps.iter().enumerate().fold(HashMap::new(), |mut map, (index, cap)| {
                    map.insert(index.to_string(), cap.map_or_else(String::new, |c| c.as_str().to_string()));
                    map
                });
                strfmt::strfmt(&replacement.to, &vars).unwrap()
            })
        })
    }
}

#[async_trait::async_trait]
impl RequestHandler for DomainConversionHandler {

    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
    ) -> ResponseInfo {
        // Check if the first query matches something we can handle
        let record =  if let Some(query) = request.queries().first() {
            match query.query_type() {
                RecordType::A | RecordType::AAAA | RecordType::ANY => {
                    // Try to match the name against a replacement
                    let new_value = self.find_replacement(query.name());
                    new_value.map( |value| {
                        // Respond with a CNAME record pointing to the new value
                        Record::from_rdata(
                            query.name().into(),
                            300,
                            RData::CNAME(CNAME(Name::from_utf8(value).unwrap())),
                        )
                    })
                },
                _ => None,
            }
        } else {
            None
        };

        // Send the response
        if record.is_some() {
            let rec = record.unwrap();
            let mr = MessageResponseBuilder::from_message_request(request).build(
                Header::response_from_request(request.header()),
                vec![&rec],
                vec![],
                vec![],
                vec![],
            );
            response_handle.send_response(mr).await.unwrap()
        } else {
            // No replacement found, respond with NXDomain
            let mr = MessageResponseBuilder::from_message_request(request).error_msg(
                request.header(),
                ResponseCode::NXDomain
            );
            response_handle.send_response(mr).await.unwrap()
        }
    }

}

async fn create_server(config: Config) -> Result<ServerFuture<DomainConversionHandler>, Box<dyn std::error::Error>> {
    // Bind to UDP port 8053 (you can change this)
    let socket = UdpSocket::bind(config.bind_address).await?;

    // Create a server
    let mut server = ServerFuture::new(DomainConversionHandler::new(config.replacements));
    server.register_socket(socket);

    Ok(server)
}


#[derive(Parser, Debug)]
#[command(name = "dns-redirect")]
#[command(about = "A simple DNS server that redirects queries based on regex replacements using cname records.")]
struct Args {
    #[arg(short, long, value_name = "FILE", default_value = "config.json", help = "Path to the JSON configuration file.")]
    config_file: String,
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>>{
    Args::command().print_help()?;

    let args = Args::parse();
    println!("");
    println!("Using config file: {}", args.config_file);
    println!("");
    let json = std::fs::read_to_string(args.config_file)?;
    let config = Config::load_from_json(&json)?;

    println!("");
    println!("Starting server on {} ...", &config.bind_address);
    println!("");

    let mut server = create_server(config).await?;

    println!("");
    println!("Server Running on ...");
    println!("");

    // Run the server
    server.block_until_done().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use core::panic;
    use std::net::SocketAddr;
    use std::str::FromStr;

    use futures::stream::{self, StreamExt};
    use hickory_proto::rr::{RData, Record, RecordType};
    use hickory_proto::xfer::Protocol;
    use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
    use hickory_resolver::name_server::TokioConnectionProvider;
    use hickory_resolver::Resolver;

    use super::*;

    fn find_free_port() -> std::io::Result<u16> {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
        let port = socket.local_addr()?.port();
        Ok(port)
    }

    fn setup_resolver(server_address: &str) -> Resolver<TokioConnectionProvider> {
        let name_server_config = NameServerConfig::new(
            SocketAddr::from_str(server_address).unwrap(),
            Protocol::Udp,
        );

        let mut resolver_config = ResolverConfig::new();
        resolver_config.add_name_server(name_server_config);

        let mut  resolver_opts = ResolverOpts::default();
        resolver_opts.recursion_desired = false;

        Resolver::builder_with_config(
            resolver_config,
            TokioConnectionProvider::default()
        )
        .with_options(resolver_opts)
        .build()
    }

    async fn test_server(replacements: Vec<Replacement>, test_cases: Vec<(&str, &str)>) {
        let address = format!("127.0.0.1:{}", find_free_port().unwrap());
        let mut server = create_server(Config::new(address.clone(), replacements)).await.unwrap();

        let resolver = setup_resolver(&address);

        let stream = stream::iter(test_cases.iter());

        // Perform lookups
        let lookup_results = stream.fold(vec![], |mut acc, (query, _)| async {
            let result = resolver.lookup(Name::from_utf8(*query).unwrap(), RecordType::ANY).await;
            acc.push(result);
            acc
        }).await;

        server.shutdown_gracefully().await.unwrap();

        // Check we got what we expected
        for ((query, expected), lookup_result) in test_cases.iter().zip(lookup_results.iter()) {
            let result = match lookup_result {
                Ok(res) => res,
                Err(e) => {
                    panic!("Failed to lookup {}: {}", query, e);
                }
            };

            let is_matching_cname = |record: &Record| {
                match record.data() {
                    RData::CNAME(cname) => cname.to_string() == *expected,
                    _ => false,
                }
            };

            assert!(result.records().iter().any(is_matching_cname), "Didn't find {} in the cnames", *expected);
        }
    }

    #[tokio::test]
    async fn test_single_mapping_returns_expected_cname() {
        test_server(vec![
            Replacement {
                from: regex::Regex::new(r"^.*$").unwrap(),
                to: "bob.lan.".to_string(),
            }
        ],
        vec![
            ("bob.mnh", "bob.lan."),
            ("alice.mnh", "bob.lan."),
            ("charlie.pod", "bob.lan."),
        ]).await;
    }

    #[tokio::test]
    async fn test_mapping_returns_expected_cnames() {
        test_server(vec![
            Replacement {
                from: regex::Regex::new(r"^(.*)\.mnh.?$").unwrap(),
                to: "{1}.lan.".to_string(),
            }
        ],
        vec![
            ("bob.mnh", "bob.lan."),
            ("alice.mnh", "alice.lan."),
            ("big.site.mnh", "big.site.lan."),
        ]).await;
    }

    #[tokio::test]
    async fn test_multiple_replacements() {
        test_server(vec![
            Replacement {
                from: regex::Regex::new(r"^(.*)\.mnh.?$").unwrap(),
                to: "{1}.lan.".to_string(),
            },
            Replacement {
                from: regex::Regex::new(r"^(.*)\.(.*)\.pod.?$").unwrap(),
                to: "{2}.{1}.pod.".to_string(),
            },
        ],
        vec![
            ("bob.mnh", "bob.lan."),
            ("alice.chad.pod", "chad.alice.pod."),
            ("big.site.mnh", "big.site.lan."),
            ("x.y.z.pod", "z.x.y.pod."), // Matching is greedy
        ]).await;
    }

    #[tokio::test]
    async fn test_no_match_returns_nxdomain() {
        let address = format!("127.0.0.1:{}", find_free_port().unwrap());
        let mut server = create_server(Config::new(address.clone(), vec![
            Replacement {
                from: regex::Regex::new(r"^(.*)\.mnh.?$").unwrap(),
                to: "dont.care.".to_string(),
            }
        ])).await.unwrap();

        let resolver = setup_resolver(&address);

        let lookup_result = resolver.lookup(Name::from_utf8("barry.net").unwrap(), RecordType::ANY).await;

        server.shutdown_gracefully().await.unwrap();

        // Check we got the expected error
        match lookup_result {
            Ok(res) => panic!("Expected NXDomain but got result: {:?}", res),
            Err(e) => assert!(e.is_nx_domain(), "Expected NXDomain but got different error: {}", e),
        };
    }

    #[tokio::test]
    async fn test_wrong_query_type_returns_nxdomain() {
        let address = format!("127.0.0.1:{}", find_free_port().unwrap());
        let mut server = create_server(Config::new(address.clone(), vec![
            Replacement {
                from: regex::Regex::new(r"^(.*)\.net.?$").unwrap(),
                to: "dont.care.".to_string(),
            }
        ])).await.unwrap();

        let resolver = setup_resolver(&address);

        let lookup_result = resolver.lookup(Name::from_utf8("barry.net").unwrap(), RecordType::CSYNC).await;

        server.shutdown_gracefully().await.unwrap();

        // Check we got the expected error
        match lookup_result {
            Ok(res) => panic!("Expected NXDomain but got result: {:?}", res),
            Err(e) => assert!(e.is_nx_domain(), "Expected NXDomain but got different error: {}", e),
        };
    }

    #[test]
    fn test_load_replacements_from_json() {
        let json = r#"
        [
            {
                "from": "^(.*)\\.mnh.?$",
                "to": "{1}.lan."
            },
            {
                "from": "^(.*)\\.(.*)\\.pod.?$",
                "to": "{2}.{1}.pod."
            }
        ]
        "#;

        let replacements: Vec<Replacement> = serde_json::from_str(json).unwrap();

        assert_eq!(replacements.len(), 2);
        assert!(replacements[0].from.is_match("bob.mnh"));
        assert_eq!(replacements[0].to, "{1}.lan.");
        assert!(replacements[1].from.is_match("alice.chad.pod"));
        assert_eq!(replacements[1].to, "{2}.{1}.pod.");
    }

    #[test]
    fn test_load_config_from_json_single_replacement() {
        let json = r#"
        {
            "bind_address": "98.99.100.101:8784",
            "replacements": [
                {
                    "from": "^(.*)\\.mnh.?$",
                    "to": "{1}.lan."
                }
            ]
        }
        "#;

        let config = Config::load_from_json(json).unwrap();
        assert_eq!(config.bind_address, "98.99.100.101:8784");
        assert_eq!(config.replacements.len(), 1);
        assert!(config.replacements[0].from.is_match("bob.mnh"));
        assert_eq!(config.replacements[0].to, "{1}.lan.");
    }

}
