mod probe;
use rustwright::{Browser, Chrome};
use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::TcpListener};
#[tokio::main]
async fn main() {
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=format!("http://{}/download",listener.local_addr().unwrap());
    let fixture=tokio::spawn(async move {
        loop { let (mut socket,_)=listener.accept().await.unwrap();tokio::spawn(async move {
            let mut bytes=[0;4096];let _=socket.read(&mut bytes).await;
            let body=b"diagnostic fixture";
            let header=format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=probe.txt\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
            socket.write_all(header.as_bytes()).await.unwrap();socket.write_all(body).await.unwrap();
        });}
    });
    let browser=Browser::launch(Chrome::at("/workspace/.rustwright-env/chrome155/chromium").headless(true)).await.unwrap();
    probe::diagnose_creation(&browser).await;
    let page=browser.new_page().await.unwrap();
    let original=page.goto(&url).await;
    assert!(original.is_err(),"Intentional download must not become a completed document navigation");
    eprintln!("Original intentional navigation failure: {original:?}");
    probe::diagnose(&browser,&page,&url).await;
    assert!(original.is_err(),"Diagnostics preserve the original failure");
    browser.close().await.unwrap();fixture.abort();
}
