use anyhow::{anyhow, bail, Result};
use lightning::routing::gossip::{NetworkGraph, NodeId, NodeInfo};
use lightning::types::features::NodeFeatures;
use lightning::util::logger::{Level, Logger, Record};
use lightning::util::ser::Readable;
use lightning_rapid_gossip_sync::RapidGossipSync;
use std::fs::File;
use std::io::{Cursor, Read};
use std::str::FromStr;
use std::sync::Arc;

struct SimpleLogger {
	level: Level,
}
impl SimpleLogger {
	fn new(level: Level) -> Self {
		Self { level }
	}
}
impl Logger for SimpleLogger {
	fn log(&self, record: Record<'_>) {
		if record.level >= self.level {
			println!("{}: {}", record.level, record.args);
		}
	}
}

fn main() -> Result<()> {
	let level = std::env::var("RUST_LOG").unwrap_or("INFO".to_string());
	let level = match level.to_uppercase().as_str() {
		"GOSSIP" => Level::Gossip,
		"TRACE" => Level::Trace,
		"DEBUG" => Level::Debug,
		"INFO" => Level::Info,
		"WARN" => Level::Warn,
		"ERROR" => Level::Error,
		other => bail!("Unknown log level: {other}"),
	};

	let mut args = std::env::args().into_iter();
	let app_name = args.next().unwrap_or_default();
	let snapshot_path = args.next().unwrap_or_default();
	if snapshot_path.is_empty() {
		println!("Usage: {app_name} <snapshot_file> [node_id]");
		return Ok(());
	}
	let node_id = args.next().map(|s| NodeId::from_str(&s)).transpose()?;

	let mut file = File::open(snapshot_path)?;
	let mut buffer = Vec::new();
	file.read_to_end(&mut buffer)?;

	let mut cursor = Cursor::new(&buffer);
	let mut protocol_prefix = [0u8; 3];
	cursor.read_exact(&mut protocol_prefix)?;
	let protocol_prefix_string = String::from_utf8(protocol_prefix.to_vec()).unwrap_or_default();
	println!("      Protocol prefix: {protocol_prefix:?} [{protocol_prefix_string}]");

	let version: u8 = Readable::read(&mut cursor).unwrap();
	println!("     Snapshot version: {version}");

	let chain_hash = Readable::read(&mut cursor).unwrap();
	let network = bitcoin::Network::from_chain_hash(chain_hash).unwrap();
	println!("              Network: {chain_hash} [{network}]");

	let latest_seen_timestamp: u32 = Readable::read(&mut cursor).unwrap();
	let latest_seen = unix_timestamp_to_utc_datetime(latest_seen_timestamp);
	println!("Latest seen timestamp: {latest_seen_timestamp} [{latest_seen}]");

	if version == 2 {
		let default_feature_count: u8 = Readable::read(&mut cursor).unwrap();
		println!("Default node feature count: {default_feature_count}");
		for i in 0..default_feature_count {
			let node_features: NodeFeatures = Readable::read(&mut cursor).unwrap();
			println!("  Default node feature #{i}: {node_features}");
		}
	}

	let node_id_count: u32 = Readable::read(&mut cursor).unwrap();
	println!("        Node id count: {node_id_count}");

	let logger = Arc::new(SimpleLogger::new(level));
	let network_graph = Arc::new(NetworkGraph::new(network, Arc::clone(&logger)));
	let rapid_sync = RapidGossipSync::new(network_graph.clone(), logger);
	let last_sync_timestamp =
		rapid_sync.update_network_graph(&buffer).map_err(|e| anyhow!("{e:?}"))?;
	println!("Snaphot successfully processed");
	let last_sync = unix_timestamp_to_utc_datetime(last_sync_timestamp);
	println!("  Last sync timestamp: {last_sync_timestamp} [{last_sync}]");

	// Print some basic information about the network graph
	let graph = network_graph.read_only();
	println!("\nNetwork Graph Statistics");
	println!("      Number of nodes: {}", graph.nodes().len());
	println!("   Number of channels: {}", graph.channels().len());
	println!();

	match node_id {
		Some(node_id) => {
			let node = graph.node(&node_id).ok_or(anyhow!("Node not found"))?;
			print_node_details(&node_id, node);
		},
		None => {
			println!("\nFirst few nodes:");
			for (node_id, node) in graph.nodes().unordered_iter().take(3) {
				print_node_details(node_id, node);
			}

			println!("\nFirst few nodes with announcement info:");
			for (node_id, node) in graph
				.nodes()
				.unordered_iter()
				.filter(|(_, node)| node.announcement_info.is_some())
				.take(3)
			{
				print_node_details(node_id, node);
			}

			println!("\nFirst few channels:");
			for (channel_id, channel) in graph.channels().unordered_iter().take(5) {
				println!("Channel: {channel_id}");
				println!("  Capacity: {:?} sats", channel.capacity_sats);
				println!("  Node 1: {}", channel.node_one);
				println!("  Node 2: {}", channel.node_two);
			}
		},
	}

	Ok(())
}

fn print_node_details(node_id: &NodeId, node: &NodeInfo) {
	println!("Node: {node_id}");
	if let Some(info) = node.announcement_info.as_ref() {
		println!("  Alias: {}", info.alias());
		println!("  Addresses:");
		for address in info.addresses() {
			println!("    - {address}");
		}
		println!("  Features: {}", info.features());
	}
	println!("  Number of channels: {}", node.channels.len());
}

fn unix_timestamp_to_utc_datetime(secs: u32) -> chrono::DateTime<chrono::Utc> {
	use chrono::{DateTime, Utc};
	let datetime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
	DateTime::<Utc>::from(datetime)
}
