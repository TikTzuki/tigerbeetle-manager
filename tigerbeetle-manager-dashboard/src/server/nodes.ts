// Node addresses configuration.
//
// Single-cluster (backward compat):
//   MANAGER_NODES=node-0:localhost:9090,node-1:localhost:9091,...
//   Node IDs are unchanged ("node-0", "node-1", ...).
//
// Multi-cluster:
//   MANAGER_CLUSTERS=prod=node-0:10.0.0.1:9090,node-1:10.0.0.2:9090|staging=node-0:10.0.1.1:9090
//   Clusters are |-separated; nodes within each cluster are ,-separated.
//   Node IDs are globally scoped: "{clusterId}~{localNodeId}" (e.g. "prod~node-0").

export interface NodeConfig {
    id: string;        // globally unique (e.g. "prod~node-0" or "node-0")
    clusterId: string; // owning cluster id
    localId: string;   // bare id within the cluster (e.g. "node-0")
    host: string;
    port: number;
}

export interface ClusterConfig {
    id: string;
    nodes: NodeConfig[];
}

export function getClusterConfigs(): ClusterConfig[] {
    const clustersEnv = process.env.MANAGER_CLUSTERS;
    if (clustersEnv) {
        return clustersEnv.split("|").map((entry) => {
            const eqIdx = entry.indexOf("=");
            const clusterId = entry.substring(0, eqIdx).trim();
            const nodes = entry.substring(eqIdx + 1).split(",").map((n) => {
                const [localId, host, port] = n.trim().split(":");
                return {
                    id: `${clusterId}~${localId}`,
                    clusterId,
                    localId,
                    host,
                    port: parseInt(port, 10),
                };
            });
            return {id: clusterId, nodes};
        });
    }

    // Backward compat: MANAGER_NODES as single "default" cluster.
    const nodesEnv = process.env.MANAGER_NODES;
    const rawNodes = nodesEnv
        ? nodesEnv.split(",").map((n) => {
            const [id, host, port] = n.trim().split(":");
            return {id, host, port: parseInt(port, 10)};
        })
        : Array.from({length: 6}, (_, i) => ({
            id: `node-${i}`,
            host: "localhost",
            port: 9090 + i,
        }));

    return [{
        id: "default",
        nodes: rawNodes.map((n) => ({
            id: n.id,
            clusterId: "default",
            localId: n.id,
            host: n.host,
            port: n.port,
        })),
    }];
}

export function getNodeConfigs(): NodeConfig[] {
    return getClusterConfigs().flatMap((c) => c.nodes);
}
