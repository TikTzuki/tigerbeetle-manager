// Node address configuration.
//
// Set MANAGER_NODES to a comma-separated list of host:port pairs:
//   MANAGER_NODES=10.0.0.1:9090,10.0.0.2:9090,10.0.0.3:9090
//
// Node IDs are auto-assigned as "node-0", "node-1", … based on position.
// Cluster membership is discovered automatically by reading the cluster_id
// from each node's superblock via GetStatus.
//
// Default (no env var): 6 nodes on localhost:9090–9095.

export interface NodeConfig {
    id: string;   // "node-0", "node-1", etc.
    host: string;
    port: number;
}

export function getNodeConfigs(): NodeConfig[] {
    const nodesEnv = process.env.MANAGER_NODES;
    if (!nodesEnv?.trim()) {
        return Array.from({length: 6}, (_, i) => ({
            id: `${i}`,
            host: "localhost",
            port: 9090 + i,
        }));
    }
    return nodesEnv.split(",").map((entry, i) => {
        const trimmed = entry.trim();
        const lastColon = trimmed.lastIndexOf(":");
        const host = trimmed.substring(0, lastColon);
        const port = parseInt(trimmed.substring(lastColon + 1), 10);
        return {id: `${i}`, host, port};
    });
}
