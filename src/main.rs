use hickory_server::authority::{Catalog, ZoneType, Authority};
use hickory_server::proto::rr::{Name, Record, RecordType, RData};
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use hickory_server::store::in_memory::InMemoryAuthority;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::net::UdpSocket;

// Define a simple request handler
#[derive(Clone)]
struct SimpleHandler {
    catalog: Catalog,
}

#[async_trait::async_trait]
impl RequestHandler for SimpleHandler {
    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response: R,
    ) -> ResponseInfo {
        self.catalog.lookup(&request.query, &mut response).await
    }
}

// Create a simple authority with hardcoded records
fn create_authority() -> InMemoryAuthority {
    let origin = Name::from_ascii("example.com.").unwrap();
    let mut authority = InMemoryAuthority::empty(origin.clone(), ZoneType::Primary, false);

    // Add an A record for example.com
    let record = Record::new()
        .set_name(origin.clone())
        .set_ttl(3600)
        .set_rr_type(RecordType::A)
        .set_rdata(RData::A(Ipv4Addr::new(192, 168, 1, 100).into()))
        .clone();

    authority.upsert(record, 0);
    authority
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up the catalog with our authority
    let mut catalog = Catalog::new();
    let authority = create_authority();
    catalog.upsert(authority.origin().clone(), Box::new(authority));

    // Create the handler
    let handler = SimpleHandler { catalog };

    // Bind to UDP port 8053 (avoid 53 to prevent root privilege issues during testing)
    let socket = UdpSocket::bind("127.0.0.1:8053").await?;
    println!("DNS server running on 127.0.0.1:8053");

    // Buffer for incoming requests
    let mut buf = vec![0; 1024];

    loop {
        // Receive DNS query
        let (len, src) = socket.recv_from(&mut buf).await?;
        let request = match hickory_server::proto::serialize::binary::BinDecoder::new(&buf[..len])
            .read_message()
        {
            Ok(msg) => msg,
            Err(_) => continue,
        };

        // Handle the request
        let response = handler.handle_request(&Request::new(request, src), &socket).await;

        // Send response back to client
        let response_bytes = response.to_bytes()?;
        socket.send_to(&response_bytes, src).await?;
    }
}