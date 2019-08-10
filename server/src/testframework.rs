//! Helper framework for writing unit tests.

use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use mio_extras::channel::{channel, Receiver, Sender};
use rand::Rng;
use specs::{Builder, Dispatcher, Entity, ReaderId, World, WorldExt};
use uuid::Uuid;

use feather_core::network::packet::{Packet, PacketType};
use feather_core::world::Position;
use feather_core::Gamemode;

use crate::config::Config;
use crate::entity::{EntityComponent, PlayerComponent};
use crate::io::ServerToHandleMessage;
use crate::network::{NetworkComponent, PacketQueue};
use crate::player::InventoryComponent;
use crate::PlayerCount;
use feather_core::level::LevelData;
use shrev::EventChannel;

/// Initializes a Specs world and dispatcher
/// using default configuration options and an
/// available server port.
pub fn init_world<'a, 'b>() -> (World, Dispatcher<'a, 'b>) {
    let mut config = Config::default();
    config.server.port = find_open_port().unwrap();

    let config = Arc::new(config);

    let player_count = Arc::new(PlayerCount(AtomicUsize::new(0)));
    let ioman = super::init_io_manager(Arc::clone(&config), Arc::clone(&player_count));
    let level = LevelData::default();

    super::init_world(config, player_count, ioman, level)
}

pub struct Player {
    pub entity: Entity,
    pub network_sender: Sender<ServerToHandleMessage>,
    pub network_receiver: Receiver<ServerToHandleMessage>,
}

/// Adds a player to the world, inserting
/// all the necessary components. Returns
/// a number of useful channels.
pub fn add_player(world: &mut World) -> Player {
    let (ns1, nr1) = channel();
    let (ns2, nr2) = channel();
    let e = world
        .create_entity()
        .with(NetworkComponent::new(ns1, nr2))
        .with(PlayerComponent {
            gamemode: Gamemode::Creative,
            profile_properties: vec![],
        })
        .with(EntityComponent {
            uuid: Uuid::new_v4(),
            on_ground: true,
            position: Position::new(0.0, 0.0, 0.0, 0.0, 0.0),
            display_name: "Test".to_string(),
        })
        .with(InventoryComponent::default())
        .build();

    Player {
        entity: e,
        network_sender: ns2,
        network_receiver: nr1,
    }
}

/// Asserts that the given player has received
/// a packet of the given type, returning the packet.
pub fn assert_packet_received(player: &Player, ty: PacketType) -> Box<dyn Packet> {
    while let Ok(msg) = player.network_receiver.try_recv() {
        if let ServerToHandleMessage::SendPacket(packet) = msg {
            if packet.ty() == ty {
                return packet;
            }
        }
    }

    panic!();
}

/// Asserts that a player did not receive
/// any packets of the given type.
/// Panics if not.
pub fn assert_packet_not_received(player: &Player, ty: PacketType) {
    while let Ok(msg) = player.network_receiver.try_recv() {
        if let ServerToHandleMessage::SendPacket(packet) = msg {
            assert_ne!(packet.ty(), ty);
        }
    }
}

/// Retrieves up to `cap` packets sent to a player, if any.
/// If `cap` is set to `None`, all packets will be read.
///
/// Note that this function consumes messages in
/// the network channel until enough packets have been read.
pub fn received_packets(player: &Player, cap: Option<usize>) -> Vec<Box<dyn Packet>> {
    let mut result = vec![];

    while let Ok(msg) = player.network_receiver.try_recv() {
        if let ServerToHandleMessage::SendPacket(pack) = msg {
            result.push(pack);
        }
        if let Some(cap) = cap.as_ref() {
            if result.len() >= *cap {
                break;
            }
        }
    }

    result
}

/// Adds a received packet to the packet queue
/// for a given player.
pub fn receive_packet<P: Packet + 'static>(player: &Player, world: &World, packet: P) {
    let queue = world.fetch_mut::<PacketQueue>();
    queue.add_for_packet(player.entity, Box::new(packet));
}

/// Attempts to find an available port.
fn find_open_port() -> Option<u16> {
    let start = rand::thread_rng().gen_range(10000, 30000);
    (start..60000).find(|port| TcpListener::bind(("127.0.0.1", *port)).is_ok())
}

/// Asserts that a player was disconnected, panicking if not.
pub fn assert_disconnected(player: &Player) {
    let mut disconnected = false;
    for packet in received_packets(player, None) {
        if packet.ty() == PacketType::DisconnectPlay {
            disconnected = true;
        }
    }

    assert!(disconnected);
}

/// Asserts that a player was not disconnected, panicking
/// if they were.
pub fn assert_not_disconnected(player: &Player) {
    let mut disconnected = false;
    for packet in received_packets(player, None) {
        if packet.ty() == PacketType::DisconnectPlay {
            disconnected = true;
        }
    }

    assert!(!disconnected);
}

/// Sends a packet to the player.
pub fn send_packet<P: Packet + 'static>(player: &Player, packet: P) {
    player
        .network_sender
        .send(ServerToHandleMessage::NotifyPacketReceived(Box::new(
            packet,
        )))
        .unwrap();
}

/// Registers a reader for events of the given type.
pub fn reader<E: Send + Sync>(w: &World) -> ReaderId<E> {
    let mut channel = w.fetch_mut::<EventChannel<E>>();
    channel.register_reader()
}

/// Triggers the given event, writing it to
/// the corresponding `EventChannel`.
pub fn trigger_event<E: Send + Sync + 'static>(world: &mut World, event: E) {
    let mut channel = world.fetch_mut::<EventChannel<E>>();
    channel.single_write(event);
}

/// Heh... tests for the testing framework.
/// Not sure what the point of this is, since
/// all other tests would fail if the testing
/// framework didn't work.
mod tests {
    use crate::entity::{EntityComponent, PlayerComponent};
    use crate::network::{send_packet_to_player, NetworkComponent};
    use feather_core::network::packet::implementation::{DisconnectPlay, LoginStart};

    use super::*;

    #[test]
    fn test_find_open_port() {
        let port = find_open_port().unwrap();
        println!("Found open port: {}", port);
        assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn test_init_world() {
        // Check that initializing the world doesn't cause
        // a panic.
        let (w, mut d) = init_world();

        // Check that running the dispatcher works fine
        d.dispatch(&w);
    }

    #[test]
    fn test_add_player() {
        let (mut w, _) = init_world();

        let entity = add_player(&mut w).entity;

        assert!(w.read_component::<PlayerComponent>().get(entity).is_some());
        assert!(w.read_component::<EntityComponent>().get(entity).is_some());
        assert!(w.read_component::<NetworkComponent>().get(entity).is_some());
    }

    #[test]
    fn test_received_packets() {
        let (mut w, _) = init_world();

        let player = add_player(&mut w);

        let cap = 1;
        send_packet_to_player(
            w.read_component().get(player.entity).unwrap(),
            LoginStart::new("".to_string()),
        );
        send_packet_to_player(
            w.read_component().get(player.entity).unwrap(),
            LoginStart::new("".to_string()),
        );

        let packets = received_packets(&player, Some(cap));
        assert_eq!(packets.len(), 1);

        let packets = received_packets(&player, Some(cap));
        assert_eq!(packets.len(), 1);
    }

    #[test]
    #[should_panic]
    fn test_assert_packet_received() {
        let (mut w, _) = init_world();

        let player = add_player(&mut w);
        assert_packet_received(&player, PacketType::Handshake);
    }

    #[test]
    #[should_panic]
    fn test_assert_disconnected() {
        let (mut w, _) = init_world();

        let player = add_player(&mut w);
        assert_disconnected(&player);
    }

    #[test]
    #[should_panic]
    fn test_assert_not_disconnected() {
        let (mut w, _) = init_world();

        let disconnect = DisconnectPlay::new("bla".to_string());

        let player = add_player(&mut w);
        send_packet_to_player(w.read_component().get(player.entity).unwrap(), disconnect);
        assert_not_disconnected(&player);
    }
}
