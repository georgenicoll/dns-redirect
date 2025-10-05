use std::collections::HashMap;

use hickory_proto::op::{Header, ResponseCode};
use hickory_proto::rr::rdata::CNAME;
use hickory_proto::rr::{LowerName, Name, RData, Record, RecordType};
use hickory_server::authority::MessageResponseBuilder;
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo, ServerFuture};
use tokio::net::UdpSocket;


#[derive(Clone)]
struct Replacement {
    from: regex::Regex,
    to: String,
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

async fn create_server(address: &str, replacements: Vec<Replacement>) -> Result<ServerFuture<DomainConversionHandler>, Box<dyn std::error::Error>> {
    // Bind to UDP port 8053 (you can change this)
    let socket = UdpSocket::bind(address).await?;

    // Create a server
    let mut server = ServerFuture::new(DomainConversionHandler::new(replacements));
    server.register_socket(socket);

    Ok(server)
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>>{
    let mut server = create_server("127.0.0.1:8053", vec![]).await?;

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
        let mut server = create_server(&address, replacements).await.unwrap();

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
        let mut server = create_server(&address, vec![
            Replacement {
                from: regex::Regex::new(r"^(.*)\.mnh.?$").unwrap(),
                to: "dont.care.".to_string(),
            }
        ]).await.unwrap();

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
        let mut server = create_server(&address, vec![
            Replacement {
                from: regex::Regex::new(r"^(.*)\.net.?$").unwrap(),
                to: "dont.care.".to_string(),
            }
        ]).await.unwrap();

        let resolver = setup_resolver(&address);

        let lookup_result = resolver.lookup(Name::from_utf8("barry.net").unwrap(), RecordType::CSYNC).await;

        server.shutdown_gracefully().await.unwrap();

        // Check we got the expected error
        match lookup_result {
            Ok(res) => panic!("Expected NXDomain but got result: {:?}", res),
            Err(e) => assert!(e.is_nx_domain(), "Expected NXDomain but got different error: {}", e),
        };
    }

}
