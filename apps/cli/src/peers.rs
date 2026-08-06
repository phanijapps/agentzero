use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use gateway_a2a::peers::{AddPeer, IssueCredential, PeerStore};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct PeersArgs {
    #[command(subcommand)]
    command: PeerCommand,
}

#[derive(Subcommand, Debug)]
enum PeerCommand {
    /// Show trusted peers and credential metadata without secret material.
    List,
    /// Discover currently visible A2A candidates.
    Discover,
    /// Issue a new inbound credential for a trusted peer.
    Issue(TokenArgs),
    /// Alias for issue; prints the new inbound credential once.
    Token(TokenArgs),
    /// Rotate by issuing one additional inbound credential.
    Rotate(TokenArgs),
    /// Revoke an inbound credential immediately.
    Revoke {
        peer_id: String,
        credential_id: String,
    },
    /// Add or update outbound trust for a peer.
    Add(AddArgs),
    /// Remove a trusted peer.
    Remove { peer_id: String },
}

#[derive(Args, Debug)]
struct TokenArgs {
    peer_id: String,
    #[arg(long)]
    display_name: Option<String>,
    #[arg(long)]
    target_agent: String,
    #[arg(long)]
    days: Option<i64>,
}

#[derive(Args, Debug)]
struct AddArgs {
    peer_id: String,
    #[arg(long)]
    origin: String,
    #[arg(long)]
    token: Option<String>,
    #[arg(long)]
    display_name: Option<String>,
    #[arg(long)]
    target_agent: String,
    #[arg(long)]
    allow_private_http: bool,
}

pub fn run(args: PeersArgs, data_dir: PathBuf) -> Result<()> {
    let store = PeerStore::new(data_dir);
    match args.command {
        PeerCommand::List => list(&store),
        PeerCommand::Discover => {
            println!("A2A discovery candidates are managed by the daemon; no local candidates available.");
            Ok(())
        }
        PeerCommand::Issue(args) | PeerCommand::Token(args) | PeerCommand::Rotate(args) => {
            let issued = store
                .issue_credential(IssueCredential {
                    peer_id: args.peer_id,
                    display_name: args.display_name,
                    target_agent_id: args.target_agent,
                    lifetime_days: args.days,
                })
                .context("issue inbound credential")?;
            println!("peer_id: {}", issued.peer_id);
            println!("credential_id: {}", issued.credential_id);
            println!("expires_at: {}", issued.expires_at.to_rfc3339());
            println!("token: {}", issued.token.exposed());
            Ok(())
        }
        PeerCommand::Revoke {
            peer_id,
            credential_id,
        } => {
            store
                .revoke_credential(&peer_id, &credential_id)
                .context("revoke inbound credential")?;
            println!("revoked {credential_id} for {peer_id}");
            Ok(())
        }
        PeerCommand::Add(args) => {
            store
                .add_peer(AddPeer {
                    peer_id: args.peer_id.clone(),
                    display_name: args.display_name.unwrap_or_else(|| args.peer_id.clone()),
                    origin: args.origin,
                    target_agent_id: args.target_agent,
                    outbound_token: args.token,
                    allow_private_http: args.allow_private_http,
                })
                .context("add peer")?;
            println!("added {}", args.peer_id);
            Ok(())
        }
        PeerCommand::Remove { peer_id } => {
            store.remove_peer(&peer_id).context("remove peer")?;
            println!("removed {peer_id}");
            Ok(())
        }
    }
}

fn list(store: &PeerStore) -> Result<()> {
    let snapshot = store.load_snapshot().context("load peer store")?;
    let redacted = snapshot.redacted();
    if redacted.peers.is_empty() {
        println!("no trusted peers");
        return Ok(());
    }
    for peer in redacted.peers {
        println!("peer_id: {}", peer.node_id);
        println!("display_name: {}", peer.display_name);
        println!("target_agent: {}", peer.target_agent_id);
        if let Some(origin) = peer.origin {
            println!("origin: {origin}");
        }
        println!("outbound_credential: {}", peer.has_outbound_credential);
        for credential in peer.inbound_credentials {
            let status = if credential.revoked_at.is_some() {
                "revoked"
            } else {
                "active"
            };
            println!(
                "inbound_credential: {} {} expires_at={}",
                credential.credential_id,
                status,
                credential.expires_at.to_rfc3339()
            );
        }
    }
    Ok(())
}
