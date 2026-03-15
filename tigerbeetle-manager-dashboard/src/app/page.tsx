"use client";

import {trpc} from "@/trpc/client";
import {useEffect, useState} from "react";
import {useRouter} from "next/navigation";

function formatUptime(seconds: string | number): string {
    const s = typeof seconds === "string" ? parseInt(seconds, 10) : seconds;
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    return `${h}h ${m}m`;
}

function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

function shortenClusterId(id: string): string {
    if (id.length <= 16) return id;
    return `${id.slice(0, 8)}…${id.slice(-6)}`;
}

function CapacityBar({capacity}: {
    capacity: { data_file_size_bytes: string; grid_blocks_total: string; grid_blocks_used: string } | null
}) {
    if (!capacity) return null;
    const total = parseInt(capacity.grid_blocks_total, 10);
    const used = parseInt(capacity.grid_blocks_used, 10);
    if (total === 0) return null;
    const pct = Math.min(100, (used / total) * 100);
    const fileSize = parseInt(capacity.data_file_size_bytes, 10);
    const color =
        pct >= 90 ? "bg-red-500" : pct >= 70 ? "bg-amber-500" : "bg-blue-500";
    return (
        <div className="mt-1.5">
            <div className="mb-0.5 flex justify-between text-xs">
                <span className="text-gray-500">Capacity</span>
                <span className="font-mono tabular-nums">{pct.toFixed(1)}% · {formatBytes(fileSize)}</span>
            </div>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-gray-200">
                <div
                    className={`h-full rounded-full transition-all ${color}`}
                    style={{width: `${pct}%`}}
                />
            </div>
        </div>
    );
}

function ProcessStateBadge({state}: { state: string }) {
    const colors: Record<string, string> = {
        PROCESS_STATE_RUNNING: "bg-green-100 text-green-800",
        PROCESS_STATE_STOPPED: "bg-gray-100 text-gray-600",
        PROCESS_STATE_CRASHED: "bg-red-100 text-red-800",
        PROCESS_STATE_STARTING: "bg-yellow-100 text-yellow-800",
    };
    const labels: Record<string, string> = {
        PROCESS_STATE_RUNNING: "Running",
        PROCESS_STATE_STOPPED: "Stopped",
        PROCESS_STATE_CRASHED: "Crashed",
        PROCESS_STATE_STARTING: "Starting",
    };
    return (
        <span
            className={`inline-block rounded-full px-2 py-0.5 text-xs font-medium ${colors[state] || "bg-gray-100 text-gray-600"}`}>
            {labels[state] || "Unknown"}
        </span>
    );
}

type NodeEntry = {
    id: string;
    host: string;
    port: number;
    online: boolean;
    status: {
        cluster_id?: string;
        replica?: number;
        replica_count?: number;
        process?: { state: string; address: string };
        backup?: { enabled: boolean; cron_schedule: string };
        uptime_seconds?: string | number;
        capacity?: { data_file_size_bytes: string; grid_blocks_total: string; grid_blocks_used: string } | null;
    } | null;
};

function NodeCard({node}: { node: NodeEntry }) {
    const router = useRouter();
    return (
        <button
            onClick={() => router.push(`/nodes/${node.id}`)}
            className={`rounded-lg border bg-white p-4 text-left transition-shadow hover:shadow-md focus:outline-none ${
                node.online ? "border-gray-200" : "border-red-200 bg-red-50/50"
            }`}
        >
            <div className="mb-3 flex items-center justify-between">
                <div>
                    <h3 className="font-mono text-sm font-semibold">
                        Replica {node.status?.replica != null && node.status.replica >= 0
                        ? node.status.replica
                        : node.id}
                    </h3>
                    {node.status?.replica_count != null && node.status.replica_count > 0 && (
                        <p className="text-xs text-gray-400">of {node.status.replica_count}</p>
                    )}
                </div>
                {node.online ? (
                    <span className="flex items-center gap-1 text-xs text-green-600">
                        <span className="inline-block h-2 w-2 rounded-full bg-green-500"/>Online
                    </span>
                ) : (
                    <span className="flex items-center gap-1 text-xs text-red-600">
                        <span className="inline-block h-2 w-2 rounded-full bg-red-500"/>Offline
                    </span>
                )}
            </div>

            {!node.online && (
                <p className="text-xs text-red-600">Cannot reach {node.host}:{node.port}</p>
            )}

            {node.online && node.status && (
                <div className="space-y-1.5 text-xs">
                    <div className="flex justify-between">
                        <span className="text-gray-500">Process</span>
                        <ProcessStateBadge
                            state={node.status.process?.state || "PROCESS_STATE_UNKNOWN"}/>
                    </div>
                    <div className="flex justify-between">
                        <span className="text-gray-500">Address</span>
                        <span className="font-mono">:{node.status.process?.address || "—"}</span>
                    </div>
                    <div className="flex justify-between">
                        <span className="text-gray-500">Uptime</span>
                        <span>{formatUptime(node.status.uptime_seconds ?? 0)}</span>
                    </div>
                    <div className="flex justify-between">
                        <span className="text-gray-500">Backups</span>
                        <span
                            className={node.status.backup?.enabled ? "font-medium text-green-600" : "text-gray-400"}>
                            {node.status.backup?.enabled ? `On · ${node.status.backup.cron_schedule}` : "Off"}
                        </span>
                    </div>
                    <CapacityBar capacity={node.status.capacity ?? null}/>
                    <p className="mt-2 text-center text-gray-400">Click to open →</p>
                </div>
            )}
        </button>
    );
}

export default function Home() {
    const [secretKey, setSecretKey] = useState("");
    const [isAuthenticated, setIsAuthenticated] = useState(false);

    const checkAuth = trpc.manager.checkAuth.useQuery();
    const login = trpc.manager.login.useMutation();
    const logout = trpc.manager.logout.useMutation();
    const allNodes = trpc.manager.getAllNodeStatuses.useQuery(undefined, {
        enabled: isAuthenticated,
        refetchInterval: 5000,
    });

    useEffect(() => {
        if (checkAuth.data?.isAuthenticated) setIsAuthenticated(true);
    }, [checkAuth.data]);

    const handleLogin = async (e: React.FormEvent) => {
        e.preventDefault();
        const result = await login.mutateAsync({secretKey});
        if (result.success) {
            setIsAuthenticated(true);
            setSecretKey("");
            checkAuth.refetch();
        }
    };

    const handleLogout = async () => {
        await logout.mutateAsync();
        setIsAuthenticated(false);
        checkAuth.refetch();
    };

    if (!isAuthenticated) {
        return (
            <main className="flex min-h-screen items-center justify-center bg-gray-50">
                <div className="w-full max-w-sm space-y-6 rounded-lg border border-gray-200 bg-white p-8 shadow-sm">
                    <div className="space-y-2 text-center">
                        <h1 className="text-2xl font-semibold tracking-tight">TigerBeetle Manager</h1>
                        <p className="text-sm text-gray-500">Cluster Dashboard — Enter admin secret key</p>
                    </div>
                    <form onSubmit={handleLogin} className="space-y-4">
                        <input
                            type="password"
                            value={secretKey}
                            onChange={(e) => setSecretKey(e.target.value)}
                            placeholder="Admin Secret Key"
                            className="w-full rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-gray-900 focus:outline-none focus:ring-1 focus:ring-gray-900"
                            required
                        />
                        <button
                            type="submit"
                            disabled={login.isPending}
                            className="w-full rounded-md bg-gray-900 px-4 py-2 text-sm font-medium text-white hover:bg-gray-800 disabled:opacity-50"
                        >
                            {login.isPending ? "Signing in..." : "Sign in"}
                        </button>
                    </form>
                    {login.isError && <p className="text-center text-sm text-red-600">Invalid secret key</p>}
                </div>
            </main>
        );
    }

    const nodes: NodeEntry[] = allNodes.data || [];
    const totalOnline = nodes.filter((n) => n.online).length;

    // Group nodes by cluster_id. Nodes without a cluster_id go to "offline" section.
    const clusterMap = new Map<string, NodeEntry[]>();
    const offlineNodes: NodeEntry[] = [];

    for (const node of nodes) {
        const cid = node.status?.cluster_id;
        if (cid) {
            if (!clusterMap.has(cid)) clusterMap.set(cid, []);
            clusterMap.get(cid)!.push(node);
        } else {
            offlineNodes.push(node);
        }
    }

    const clusters = Array.from(clusterMap.entries());

    return (
        <main className="min-h-screen bg-gray-50">
            <div className="mx-auto max-w-6xl p-6">
                {/* Header */}
                <div className="mb-6 flex items-center justify-between">
                    <div>
                        <h1 className="text-2xl font-semibold">TigerBeetle Manager</h1>
                        <p className="text-sm text-gray-500">
                            {totalOnline}/{nodes.length} nodes online
                            · {clusters.length} cluster{clusters.length !== 1 ? "s" : ""}
                        </p>
                    </div>
                    <button
                        onClick={handleLogout}
                        className="rounded-md border border-gray-300 bg-white px-4 py-2 text-sm hover:bg-gray-50"
                    >
                        Sign out
                    </button>
                </div>

                {allNodes.isLoading && (
                    <div className="rounded-lg border border-gray-200 bg-white p-8 text-center">
                        <p className="text-sm text-gray-500">Connecting to nodes...</p>
                    </div>
                )}

                {/* Clusters */}
                {clusters.map(([clusterId, clusterNodes]) => {
                    const onlineCount = clusterNodes.filter((n) => n.online).length;
                    return (
                        <div key={clusterId} className="mb-8">
                            <div className="mb-3 flex items-center gap-3">
                                <h2 className="font-mono text-sm font-semibold text-gray-700">
                                    Cluster {shortenClusterId(clusterId)}
                                </h2>
                                <span
                                    className="text-xs text-gray-400">{onlineCount}/{clusterNodes.length} online</span>
                                <span className="font-mono text-xs text-gray-300" title={clusterId}>{clusterId}</span>
                            </div>
                            <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                                {clusterNodes.map((node) => (
                                    <NodeCard key={node.id} node={node}/>
                                ))}
                            </div>
                        </div>
                    );
                })}

                {/* Offline / unreachable nodes (no cluster_id) */}
                {offlineNodes.length > 0 && (
                    <div className="mb-8">
                        <div className="mb-3 flex items-center gap-3">
                            <h2 className="text-sm font-semibold text-gray-500">Offline / Unreachable</h2>
                            <span
                                className="text-xs text-gray-400">{offlineNodes.length} node{offlineNodes.length !== 1 ? "s" : ""}</span>
                        </div>
                        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                            {offlineNodes.map((node) => (
                                <NodeCard key={node.id} node={node}/>
                            ))}
                        </div>
                    </div>
                )}

                <div className="mt-6 rounded-lg border border-amber-200 bg-amber-50 p-3">
                    <p className="text-xs text-amber-900">
                        <strong>Timezone:</strong> All cron schedules run in UTC. Convert your local time accordingly.
                    </p>
                </div>
            </div>
        </main>
    );
}
