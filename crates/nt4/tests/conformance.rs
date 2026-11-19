//! Side-by-side conformance: what our client and server do, compared with
//! what real ntcore (pyntcore) does in the same situation.
//!
//! Written by Claude. Each scenario is a list of steps (the robot sets a
//! value, the dashboard sets one, the server restarts, ...), run four times:
//! on an ntcore server and on our [`Server`], each with an ntcore client and
//! with our [`Client`] as the dashboard. A run has one dashboard, so nothing
//! it sees can come from another dashboard's writes. At each `Check`, what
//! the dashboard and the robot (the server's own code) see in the all-ntcore
//! run is the reference, settled first; the other runs must reach the same
//! state. Nobody writes down expected values: a difference from ntcore is a
//! failure, named with its step, its run and both values.
//!
//! Needs `NT4_INTEROP_PYTHON` (a Python with pyntcore 2027), like the other
//! interop tests; without it the tests print a note and pass.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use nt4::{
    Client, ClientConfig, GenericPublisher, NtType, Properties, Server, ServerConfig, Subscriber,
    Value,
};
use serde_json::{Value as Json, json};

#[path = "interop/peer.rs"]
mod peer;
use peer::{Peer, free_port};

/// A value in a scenario, with its NT type.
#[derive(Debug, Clone, Copy)]
enum V {
    B(bool),
    D(f64),
    I(i64),
    S(&'static str),
}

impl V {
    fn ty(self) -> &'static str {
        match self {
            V::B(_) => "boolean",
            V::D(_) => "double",
            V::I(_) => "int",
            V::S(_) => "string",
        }
    }
    fn json(self) -> Json {
        match self {
            V::B(b) => json!(b),
            V::D(d) => json!(d),
            V::I(i) => json!(i),
            V::S(s) => json!(s),
        }
    }
    fn value(self) -> Value {
        match self {
            V::B(b) => Value::Boolean(b),
            V::D(d) => Value::Double(d),
            V::I(i) => Value::Int(i),
            V::S(s) => Value::String(s.into()),
        }
    }
}

/// One step of a scenario.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// The robot (the server's own code) publishes and sets a value.
    Robot(&'static str, V),
    /// The robot stops publishing a topic.
    RobotUnpublish(&'static str),
    /// Both dashboard clients publish and set a value.
    Dash(&'static str, V),
    /// Both dashboard clients set a default.
    DashDefault(&'static str, V),
    /// Both dashboard clients stop publishing a topic.
    DashUnpublish(&'static str),
    /// The server goes away (robot code stopped).
    StopServer,
    /// A new server starts on the same port (robot code started again).
    StartServer,
    /// Everyone's view of a topic must match ntcore's.
    Check(&'static str),
}

/// What a viewer sees of a topic: its value, or null for none.
type Seen = Json;

/// A typed subscriber of ours, for the scenario types.
enum Sub {
    B(Subscriber<bool>),
    D(Subscriber<f64>),
    I(Subscriber<i64>),
    S(Subscriber<String>),
}

impl Sub {
    fn seen(&self) -> Seen {
        match self {
            Sub::B(s) => s.get().map_or(Json::Null, |v| json!(v)),
            Sub::D(s) => s.get().map_or(Json::Null, |v| json!(v)),
            Sub::I(s) => s.get().map_or(Json::Null, |v| json!(v)),
            Sub::S(s) => s.get().map_or(Json::Null, |v| json!(v)),
        }
    }
}

/// Our client, or our server's own code: publishers and subscribers by
/// topic name.
#[derive(Default)]
struct Ours {
    pubs: HashMap<String, GenericPublisher>,
    subs: HashMap<String, Sub>,
}

/// Our server, as the robot's NT server.
struct OurServer {
    server: Server,
    robot: Ours,
}

/// The server side of a run: ntcore's or ours.
enum ServerSide {
    Ntcore(Box<Peer>),
    Ours(Box<OurServer>),
    /// Stopped (between `StopServer` and `StartServer`).
    Down,
}

/// The dashboard of a run: an ntcore client, or ours.
enum Dashboard {
    Ntcore(Box<Peer>),
    Ours(Client, Ours),
}

/// One run of a scenario: a server and one dashboard.
struct Run {
    ntcore_server: bool,
    port: u16,
    server: ServerSide,
    dash: Dashboard,
    /// Each topic's type, so viewers subscribe with the right one.
    types: HashMap<String, &'static str>,
}

fn subscribe_ours(
    subs: &mut HashMap<String, Sub>,
    name: &str,
    ty: &str,
    client: Option<&Client>,
    server: Option<&Server>,
) {
    if subs.contains_key(name) {
        return;
    }
    macro_rules! sub {
        ($t:ty) => {
            match (client, server) {
                (Some(c), _) => c.subscribe::<$t>(name, Default::default()),
                (_, Some(s)) => s.subscribe::<$t>(name, Default::default()),
                _ => unreachable!(),
            }
        };
    }
    let sub = match ty {
        "boolean" => Sub::B(sub!(bool)),
        "double" => Sub::D(sub!(f64)),
        "int" => Sub::I(sub!(i64)),
        "string" => Sub::S(sub!(String)),
        other => panic!("scenario type {other} not supported"),
    };
    subs.insert(name.to_owned(), sub);
}

fn nt_type(ty: &str) -> NtType {
    ty.parse().unwrap()
}

impl Run {
    async fn start(ntcore_server: bool, ntcore_client: bool) -> Option<Run> {
        let port = free_port();
        let server = start_server(ntcore_server, port).await?;
        let dash = if ntcore_client {
            let mut peer = Peer::spawn().await?;
            peer.start_client("dash", port).await;
            Dashboard::Ntcore(Box::new(peer))
        } else {
            let mut config = ClientConfig::server("dash", "127.0.0.1", port);
            config.reconnect_delay = Duration::from_millis(100);
            Dashboard::Ours(Client::start(config), Ours::default())
        };
        let mut run = Run {
            ntcore_server,
            port,
            server,
            dash,
            types: HashMap::new(),
        };
        run.wait_connected().await;
        Some(run)
    }

    async fn wait_connected(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let connected = match &mut self.dash {
                Dashboard::Ntcore(peer) => peer.connected().await,
                Dashboard::Ours(client, _) => client.is_connected(),
            };
            if connected {
                return;
            }
            assert!(Instant::now() < deadline, "the dashboard never connected");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn which(&self) -> String {
        format!(
            "{} server, {} client",
            if self.ntcore_server { "ntcore" } else { "our" },
            if matches!(self.dash, Dashboard::Ntcore(_)) {
                "ntcore"
            } else {
                "our"
            },
        )
    }

    /// The dashboard publishes `name` (once) and sets `v`, or sets it as a
    /// default.
    async fn dash_set(&mut self, name: &str, v: V, default: bool) {
        self.types.insert(name.into(), v.ty());
        match &mut self.dash {
            Dashboard::Ntcore(peer) => {
                let cmd = if default { "set_default" } else { "publish" };
                peer.cmd(json!({"cmd": cmd, "name": name, "type": v.ty(), "value": v.json()}))
                    .await;
            }
            Dashboard::Ours(client, ours) => {
                let p = ours.pubs.entry(name.into()).or_insert_with(|| {
                    client.publish_generic(
                        name,
                        nt_type(v.ty()),
                        Properties::new(),
                        Default::default(),
                    )
                });
                p.set_at(v.value(), if default { 0 } else { client.now_us() })
                    .unwrap();
            }
        }
    }

    async fn step(&mut self, step: Step) {
        match step {
            Step::Robot(name, v) => {
                self.types.insert(name.into(), v.ty());
                match &mut self.server {
                    ServerSide::Ntcore(peer) => {
                        peer.cmd(json!({"cmd": "publish", "name": name, "type": v.ty(), "value": v.json()}))
                            .await;
                    }
                    ServerSide::Ours(s) => {
                        let p = s.robot.pubs.entry(name.into()).or_insert_with(|| {
                            s.server
                                .publish_generic(name, nt_type(v.ty()), Properties::new())
                        });
                        p.set(v.value()).unwrap();
                    }
                    ServerSide::Down => panic!("Robot step while the server is down"),
                }
            }
            Step::RobotUnpublish(name) => match &mut self.server {
                ServerSide::Ntcore(peer) => {
                    peer.cmd(json!({"cmd": "unpublish", "name": name})).await;
                }
                ServerSide::Ours(s) => {
                    s.robot.pubs.remove(name);
                }
                ServerSide::Down => panic!("RobotUnpublish while the server is down"),
            },
            Step::Dash(name, v) => self.dash_set(name, v, false).await,
            Step::DashDefault(name, v) => self.dash_set(name, v, true).await,
            Step::DashUnpublish(name) => match &mut self.dash {
                Dashboard::Ntcore(peer) => {
                    peer.cmd(json!({"cmd": "unpublish", "name": name})).await;
                }
                Dashboard::Ours(_, ours) => {
                    ours.pubs.remove(name);
                }
            },
            Step::StopServer => {
                // Dropping the peer kills its process; dropping our server
                // closes every connection.
                self.server = ServerSide::Down;
            }
            Step::StartServer => {
                self.server = start_server(self.ntcore_server, self.port)
                    .await
                    .expect("restart");
                self.wait_connected().await;
            }
            Step::Check(_) => unreachable!("checks are handled by the runner"),
        }
    }

    /// What the dashboard and the robot see of `name` now. The robot sees
    /// null while the server is down.
    async fn seen(&mut self, name: &str) -> (Seen, Seen) {
        let ty = self.types.get(name).copied().unwrap_or("double");
        let dash = match &mut self.dash {
            Dashboard::Ntcore(peer) => {
                peer.cmd(json!({"cmd": "peek", "name": name})).await["value"].clone()
            }
            Dashboard::Ours(client, ours) => {
                subscribe_ours(&mut ours.subs, name, ty, Some(client), None);
                ours.subs[name].seen()
            }
        };
        let robot = match &mut self.server {
            ServerSide::Ntcore(peer) => {
                peer.cmd(json!({"cmd": "peek", "name": name})).await["value"].clone()
            }
            ServerSide::Ours(s) => {
                let OurServer { server, robot } = s.as_mut();
                subscribe_ours(&mut robot.subs, name, ty, None, Some(server));
                robot.subs[name].seen()
            }
            ServerSide::Down => Json::Null,
        };
        (dash, robot)
    }
}

async fn start_server(on_ntcore: bool, port: u16) -> Option<ServerSide> {
    if on_ntcore {
        let mut peer = Peer::spawn().await?;
        peer.port4 = port;
        peer.start_server().await;
        Some(ServerSide::Ntcore(Box::new(peer)))
    } else {
        // Skip our server too when there's no ntcore to compare with.
        std::env::var("NT4_INTEROP_PYTHON").ok()?;
        let mut tries = 0;
        let server = loop {
            match Server::start(ServerConfig {
                listen_address: "127.0.0.1".parse().unwrap(),
                port,
                ..Default::default()
            })
            .await
            {
                Ok(s) => break s,
                // The old server's port may take a moment to free up.
                Err(e) if tries < 50 => {
                    tries += 1;
                    let _ = e;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => panic!("our server on port {port}: {e}"),
            }
        };
        Some(ServerSide::Ours(Box::new(OurServer {
            server,
            robot: Ours::default(),
        })))
    }
}

/// Floats and ints compare by value (`2` and `2.0` are the same double).
fn same(a: &Json, b: &Json) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// How long a viewer may take to match, and how long ntcore's view must stay
/// unchanged to count as settled.
const MATCH_WITHIN: Duration = Duration::from_secs(4);
const SETTLED_FOR: Duration = Duration::from_millis(400);

/// Run `steps` four ways. Returns every difference from the all-ntcore run.
async fn run_scenario(steps: &[Step]) -> Option<Vec<String>> {
    let mut problems = Vec::new();
    // The reference: (dashboard, robot) at each check, all ntcore.
    let mut reference: Vec<(Seen, Seen)> = Vec::new();
    for (ntcore_server, ntcore_client) in
        [(true, true), (true, false), (false, true), (false, false)]
    {
        let reference_run = ntcore_server && ntcore_client;
        let mut run = Run::start(ntcore_server, ntcore_client).await?;
        let mut check = 0;
        for (i, &step) in steps.iter().enumerate() {
            let Step::Check(name) = step else {
                run.step(step).await;
                continue;
            };
            if reference_run {
                // Wait for ntcore's view to settle, and take it as the answer.
                let start = Instant::now();
                let mut last = run.seen(name).await;
                let mut since = Instant::now();
                while since.elapsed() < SETTLED_FOR && start.elapsed() < MATCH_WITHIN {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    let now = run.seen(name).await;
                    if !(same(&now.0, &last.0) && same(&now.1, &last.1)) {
                        since = Instant::now();
                    }
                    last = now;
                }
                reference.push(last);
                check += 1;
                continue;
            }
            let (want_dash, want_robot) = reference[check].clone();
            check += 1;
            let deadline = Instant::now() + MATCH_WITHIN;
            loop {
                let (dash, robot) = run.seen(name).await;
                let ok_dash = same(&dash, &want_dash);
                let ok_robot = same(&robot, &want_robot);
                if ok_dash && ok_robot {
                    break;
                }
                if Instant::now() > deadline {
                    let mut msg = format!(
                        "step {i} Check({name}), {}: ntcore has dashboard {want_dash}, robot {want_robot}; here",
                        run.which()
                    );
                    if !ok_dash {
                        let _ = write!(msg, " the dashboard saw {dash}");
                    }
                    if !ok_robot {
                        let _ = write!(msg, " the robot saw {robot}");
                    }
                    problems.push(msg);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
    Some(problems)
}

/// Run a scenario and fail with every difference from ntcore.
async fn conforms(steps: &[Step]) {
    let Some(problems) = run_scenario(steps).await else {
        return;
    };
    assert!(
        problems.is_empty(),
        "differs from ntcore:\n  {}",
        problems.join("\n  ")
    );
}

use Step::*;
use V::*;

// ----- Scenarios -----

/// A dashboard flips a value the robot published, and reads it back.
#[tokio::test(flavor = "multi_thread")]
async fn a_dashboard_write_to_a_robot_topic() {
    conforms(&[
        Robot("/robot/enabled", B(false)),
        Check("/robot/enabled"),
        Dash("/robot/enabled", B(true)),
        Check("/robot/enabled"),
        Robot("/robot/enabled", B(false)),
        Check("/robot/enabled"),
    ])
    .await;
}

/// A dashboard's own topic, nobody else publishing it.
#[tokio::test(flavor = "multi_thread")]
async fn a_dashboards_own_topic() {
    conforms(&[
        Check("/dash/target"),
        Dash("/dash/target", D(1.5)),
        Check("/dash/target"),
        Dash("/dash/target", D(2.5)),
        Check("/dash/target"),
    ])
    .await;
}

/// Defaults: only fill an empty topic.
#[tokio::test(flavor = "multi_thread")]
async fn defaults() {
    conforms(&[
        Robot("/robot/speed", D(1.0)),
        Check("/robot/speed"),
        DashDefault("/robot/speed", D(5.0)),
        Check("/robot/speed"),
        DashDefault("/dash/fresh", D(5.0)),
        Check("/dash/fresh"),
        Dash("/dash/fresh", D(6.0)),
        Check("/dash/fresh"),
    ])
    .await;
}

/// The robot's code stops and starts again: what's left of its values and
/// of the dashboard's.
#[tokio::test(flavor = "multi_thread")]
async fn the_robot_restarts() {
    conforms(&[
        Robot("/robot/speed", D(2.0)),
        Dash("/dash/target", D(5.0)),
        Check("/robot/speed"),
        Check("/dash/target"),
        StopServer,
        Check("/robot/speed"),
        Check("/dash/target"),
        StartServer,
        Check("/robot/speed"),
        Check("/dash/target"),
        Robot("/robot/speed", D(3.0)),
        Check("/robot/speed"),
    ])
    .await;
}

/// A dashboard writes while the robot is down; it shows at once and reaches
/// the robot when it's back.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_while_the_robot_is_down() {
    conforms(&[
        StopServer,
        Dash("/dash/early", I(7)),
        Check("/dash/early"),
        StartServer,
        Check("/dash/early"),
    ])
    .await;
}

/// Stopping publishing: the robot, then a dashboard.
#[tokio::test(flavor = "multi_thread")]
async fn unpublishing() {
    conforms(&[
        Robot("/robot/mode", S("auto")),
        Check("/robot/mode"),
        RobotUnpublish("/robot/mode"),
        Check("/robot/mode"),
        Dash("/dash/note", S("hello")),
        Check("/dash/note"),
        DashUnpublish("/dash/note"),
        Check("/dash/note"),
    ])
    .await;
}

/// Several restarts in a row, with writes from both sides between them.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_restarts() {
    let mut steps = Vec::new();
    for (i, v) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        steps.extend([
            Robot("/robot/count", I(i as i64)),
            Dash("/dash/level", D(v)),
            Check("/robot/count"),
            Check("/dash/level"),
            StopServer,
            Check("/robot/count"),
            Check("/dash/level"),
            StartServer,
            Check("/dash/level"),
        ]);
    }
    conforms(&steps).await;
}
