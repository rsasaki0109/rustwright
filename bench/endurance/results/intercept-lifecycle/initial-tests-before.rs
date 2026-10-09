//! Protocol regressions for page-owned interception registrations.
use crate::{BidiBrowser,BidiError};
use futures_util::{SinkExt,StreamExt};
use rustwright_common::RouteAction;
use serde_json::{json,Value};
use std::{collections::{HashMap,HashSet},sync::{Arc,Mutex},time::Duration};
use tokio::{net::TcpListener,sync::Notify,task::JoinHandle};
use tokio_tungstenite::{accept_async,tungstenite::Message};

#[derive(Default)]
struct State { ids: HashSet<String>, methods: HashMap<String,usize>, fail: Option<&'static str>, pause: Option<&'static str> }
struct Remote { state: Arc<Mutex<State>>, entered: Arc<Notify>, release: Arc<Notify>, task:JoinHandle<()> }
impl Drop for Remote { fn drop(&mut self){self.task.abort();} }
impl Remote {
 fn count(&self)->usize{self.state.lock().unwrap().ids.len()}
 fn calls(&self,m:&str)->usize{*self.state.lock().unwrap().methods.get(m).unwrap_or(&0)}
 fn fail(&self,m:&'static str){self.state.lock().unwrap().fail=Some(m)}
 fn pause(&self,m:&'static str){self.state.lock().unwrap().pause=Some(m)}
 async fn entered(&self){tokio::time::timeout(Duration::from_secs(2),self.entered.notified()).await.unwrap();}
 async fn count_is(&self,n:usize){tokio::time::timeout(Duration::from_secs(2),async{while self.count()!=n {tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();}
}
async fn remote()->(Remote,BidiBrowser){
 let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
 let endpoint=format!("ws://{}/session",listener.local_addr().unwrap());
 let state=Arc::new(Mutex::new(State::default()));let remote=state.clone();
 let entered=Arc::new(Notify::new());let signal=entered.clone();let release=Arc::new(Notify::new());let resume=release.clone();
 let task=tokio::spawn(async move{
  let(socket,_)=listener.accept().await.unwrap();let mut ws=accept_async(socket).await.unwrap();let mut next=0;
  let mut held=None;let mut pages=HashMap::<String,String>::new();
  loop {tokio::select!{
   _=resume.notified(),if held.is_some()=>{ws.send(Message::text(held.take().unwrap())).await.unwrap();}
   message=ws.next()=>{let Some(Ok(message))=message else {break};if !message.is_text(){continue}
    let command:Value=serde_json::from_str(message.to_text().unwrap()).unwrap();let method=command["method"].as_str().unwrap();let p=&command["params"];
    let (fail,pause)={let mut s=remote.lock().unwrap();*s.methods.entry(method.to_owned()).or_default()+=1;let fail=s.fail==Some(method);let pause=s.pause==Some(method);if fail{s.fail=None}if pause{s.pause=None}(fail,pause)};
    let reply=if fail {json!({"id":command["id"],"type":"error","error":"unknown error","message":"fixture rejection"})}else{
     let result=match method{
      "session.new"=>json!({"sessionId":"mock","capabilities":{}}),
      "browser.createUserContext"=>json!({"userContext":"owned"}),
      "browsingContext.create"=>{next+=1;let tab=format!("tab-{next}");pages.insert(tab.clone(),p["userContext"].as_str().unwrap_or("default").to_owned());json!({"context":tab})},
      "browsingContext.getTree"=>json!({"contexts":pages.iter().map(|(id,owner)|json!({"context":id,"userContext":owner,"children":[]})).collect::<Vec<_>>() }),
      "script.addPreloadScript"=>{next+=1;json!({"script":format!("helper-{next}")})},
      "script.evaluate"=>json!({"type":"success","result":{"type":"undefined"}}),
      "network.addIntercept"=>{next+=1;let id=format!("intercept-{next}");remote.lock().unwrap().ids.insert(id.clone());json!({"intercept":id})},
      "network.removeIntercept"=>{remote.lock().unwrap().ids.remove(p["intercept"].as_str().unwrap());json!({})},
      "browsingContext.close"=>{pages.remove(p["context"].as_str().unwrap());json!({})},
      "browser.removeUserContext"=>{pages.retain(|_,owner|Some(owner.as_str())!=p["userContext"].as_str());json!({})},
      "session.subscribe"|"script.removePreloadScript"|"session.end"|"network.continueRequest"|"network.continueResponse"|"network.failRequest"=>json!({}),
      other=>panic!("unexpected method {other}")
     };json!({"id":command["id"],"type":"success","result":result})
    }.to_string();
    if pause {held=Some(reply);signal.notify_one();}else if ws.send(Message::text(reply)).await.is_err(){break}
   }
  }}
 });let browser=BidiBrowser::connect(&endpoint).await.unwrap();(Remote{state,entered,release,task},browser)
}
#[tokio::test]
async fn rejected_registration_is_retryable(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();r.fail("network.addIntercept");assert!(p.block("*one*").await.is_err());p.block("*two*").await.unwrap();assert_eq!(r.count(),1);assert_eq!(r.calls("network.addIntercept"),2);p.clear_routes().await.unwrap();}
#[tokio::test]
async fn removal_error_is_typed_and_retains_the_id(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();p.block("*").await.unwrap();r.fail("network.removeIntercept");assert!(matches!(p.clear_routes().await,Err(BidiError::Protocol{..})));assert_eq!(r.count(),1);p.clear_routes().await.unwrap();assert_eq!(r.count(),0);}
#[tokio::test]
async fn canceled_registration_reclaims_the_late_id(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();r.pause("network.addIntercept");let c=p.clone();let task=tokio::spawn(async move{c.block("*").await});r.entered().await;task.abort();assert!(task.await.unwrap_err().is_cancelled());r.release.notify_one();r.count_is(0).await;p.block("*").await.unwrap();assert_eq!(r.count(),1);p.clear_routes().await.unwrap();}
#[tokio::test]
async fn clear_continues_after_waiter_cancellation(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();p.block("*").await.unwrap();r.pause("network.removeIntercept");let c=p.clone();let task=tokio::spawn(async move{c.clear_routes().await});r.entered().await;task.abort();assert!(task.await.unwrap_err().is_cancelled());r.release.notify_one();tokio::time::sleep(Duration::from_millis(30)).await;p.block("*").await.unwrap();assert_eq!(r.count(),1);p.clear_routes().await.unwrap();assert_eq!(r.count(),0);}
#[tokio::test]
async fn concurrent_registration_does_not_return_before_ack(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();r.pause("network.addIntercept");let c=p.clone();let first=tokio::spawn(async move{c.block("*one*").await});r.entered().await;let c=p.clone();let second=tokio::spawn(async move{c.block("*two*").await});tokio::time::sleep(Duration::from_millis(30)).await;assert!(!second.is_finished());r.release.notify_one();first.await.unwrap().unwrap();second.await.unwrap().unwrap();assert_eq!(r.count(),1);p.clear_routes().await.unwrap();}
#[tokio::test]
async fn final_routed_page_handle_drop_removes_registration(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();p.block("*").await.unwrap();drop(p);r.count_is(0).await;assert_eq!(b.pages().await.unwrap().len(),1);}
#[tokio::test]
async fn discovered_handles_share_routes_and_keep_registration_alive(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();p.block("*one*").await.unwrap();let discovered=b.pages().await.unwrap().pop().unwrap();discovered.block("*two*").await.unwrap();assert_eq!(r.count(),1);drop(p);tokio::time::sleep(Duration::from_millis(20)).await;assert_eq!(r.count(),1);discovered.clear_routes().await.unwrap();assert_eq!(r.count(),0);}
#[tokio::test]
async fn page_close_retires_routes_while_handle_is_retained(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();p.block("*").await.unwrap();p.close().await.unwrap();assert_eq!(r.count(),0);assert!(p.block("*").await.is_err());}
#[tokio::test]
async fn user_context_close_retires_only_owned_routes(){let(r,b)=remote().await;let keeper=b.new_page().await.unwrap();keeper.block("*").await.unwrap();let c=b.new_context().await.unwrap();let p=c.new_page().await.unwrap();p.block("*").await.unwrap();c.close().await.unwrap();assert_eq!(r.count(),1);assert!(p.block("*").await.is_err());keeper.clear_routes().await.unwrap();}
#[tokio::test]
async fn failed_phase_expansion_preserves_existing_registration(){let(r,b)=remote().await;let p=b.new_page().await.unwrap();p.block("*one*").await.unwrap();r.fail("network.removeIntercept");assert!(matches!(p.route("*two*",RouteAction::SetResponseHeaders(vec![])).await,Err(BidiError::Protocol{..})));assert_eq!(r.count(),1);p.clear_routes().await.unwrap();assert_eq!(r.count(),0);}
