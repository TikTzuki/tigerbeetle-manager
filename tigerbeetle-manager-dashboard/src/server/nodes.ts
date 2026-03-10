// Node addresses configuration.
// In production, these come from environment variables.
// Format: NODES=node-0:localhost:9090,node-1:localhost:9091,...

export interface NodeConfig {
    id: string;
    host: string;
    port: number;
}

export function getNodeConfigs(): NodeConfig[] {
    const nodesEnv = process.env.MANAGER_NODES;
    if (nodesEnv) {
        return nodesEnv.split(",").map((entry) => {
            const [id, host, port] = entry.trim().split(":");
            return {id, host, port: parseInt(port, 10)};
        });
    }

    // Default: 6 nodes on localhost, ports 9090-9095.
    return Array.from({length: 6}, (_, i) => ({
        id: `node-${i}`,
        host: "localhost",
        port: 9090 + i,
    }));
}
