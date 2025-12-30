//! Port forwarding management for pods
//!
//! This module handles kubectl port-forward operations via the Kubernetes API.

use crate::sections::network::{PortForward, PortForwardStatus};
use anyhow::{Result, anyhow};
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::Api;
use std::collections::HashMap;
use tokio::io::AsyncWriteExt;
use tokio::io::copy_bidirectional;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;

/// Pipe data bidirectionally between two async streams until one side closes.
async fn pipe_streams<S1, S2>(a: &mut S1, b: &mut S2) -> Result<()>
where
    S1: AsyncRead + AsyncWrite + Unpin,
    S2: AsyncRead + AsyncWrite + Unpin,
{
    let _ = copy_bidirectional(a, b).await?;
    Ok(())
}

#[derive(Debug)]
pub struct PortForwardManager {
    /// Active port forward tunnels, keyed by tunnel_id
    pub tunnels: HashMap<String, PortForward>,
    /// Task handles for each tunnel (for cleanup)
    tunnel_tasks: HashMap<String, tokio::task::JoinHandle<()>>,
    next_tunnel_id: u64,
}

impl PortForwardManager {
    pub fn new() -> Self {
        Self {
            tunnels: HashMap::new(),
            tunnel_tasks: HashMap::new(),
            next_tunnel_id: 1,
        }
    }

    /// Start a new port forward tunnel
    /// Returns the tunnel ID
    pub async fn start(
        &mut self,
        client: &Client,
        namespace: &str,
        pod: &str,
        local_port: u16,
        remote_port: u16,
        container: Option<String>,
    ) -> Result<String> {
        let tunnel_id = format!("pf-{}", self.next_tunnel_id);
        self.next_tunnel_id += 1;

        let mut tunnel = PortForward {
            id: tunnel_id.clone(),
            namespace: namespace.to_string(),
            pod: pod.to_string(),
            local_port,
            remote_port,
            container: container.clone(),
            status: PortForwardStatus::Connecting,
        };

        // Spawn port forward task
        let pf = self.spawn_port_forward(
            client.clone(),
            namespace.to_string(),
            pod.to_string(),
            local_port,
            remote_port,
            container,
        );

        match pf.await {
            Ok(handle) => {
                tunnel.status = PortForwardStatus::Active;
                self.tunnels.insert(tunnel_id.clone(), tunnel);
                self.tunnel_tasks.insert(tunnel_id.clone(), handle);
                Ok(tunnel_id)
            }
            Err(e) => {
                tunnel.status = PortForwardStatus::Error;
                self.tunnels.insert(tunnel_id.clone(), tunnel);
                Err(e)
            }
        }
    }

    /// Spawn the actual port forwarding task
    async fn spawn_port_forward(
        &self,
        client: Client,
        namespace: String,
        pod: String,
        local_port: u16,
        remote_port: u16,
        _container: Option<String>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        // Start listening on local port
        let listener = TcpListener::bind(format!("127.0.0.1:{}", local_port))
            .await
            .map_err(|e| anyhow!("Failed to bind to local port {}: {}", local_port, e))?;

        let handle = tokio::spawn(async move {
            // Set up port forward with kube API
            let api: Api<Pod> = Api::namespaced(client, &namespace);

            loop {
                match listener.accept().await {
                    Ok((mut socket, _addr)) => {
                        let api = api.clone();
                        let pod_name = pod.clone();

                        // Handle each accepted connection with its own portforward session
                        tokio::spawn(async move {
                            match api.portforward(&pod_name, &[remote_port]).await {
                                Ok(mut forwarder) => {
                                    if let Some(mut stream) = forwarder.take_stream(remote_port) {
                                        // Pipe local TCP socket <-> pod stream until EOF
                                        let _ = pipe_streams(&mut socket, &mut stream).await;
                                    }

                                    // Ensure background task is cleaned up
                                    forwarder.abort();
                                    let _ = forwarder.join().await;
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Port-forward to pod {}:{} failed: {}",
                                        pod_name,
                                        remote_port,
                                        e
                                    );
                                    let _ = socket.write_all(b"Port forward error\n").await;
                                }
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Error accepting connection on port {}: {}", local_port, e);
                        break;
                    }
                }
            }
        });

        Ok(handle)
    }

    /// Stop a port forward tunnel
    pub async fn stop(&mut self, tunnel_id: &str) -> Result<()> {
        // Abort the task if it exists
        if let Some(task) = self.tunnel_tasks.remove(tunnel_id) {
            task.abort();
        }

        // Mark tunnel as closed
        if let Some(tunnel) = self.tunnels.get_mut(tunnel_id) {
            tunnel.status = PortForwardStatus::Closed;
        } else {
            self.tunnels.remove(tunnel_id);
        }

        Ok(())
    }

    /// Get all active tunnels
    pub fn list(&self) -> Vec<PortForward> {
        self.tunnels.values().cloned().collect()
    }

    /// Cleanup all tunnels (called on shutdown)
    pub fn cleanup_all(&mut self) {
        for (_id, task) in self.tunnel_tasks.drain() {
            task.abort();
        }
        self.tunnels.clear();
    }
}

impl Default for PortForwardManager {
    fn default() -> Self {
        Self::new()
    }
}
