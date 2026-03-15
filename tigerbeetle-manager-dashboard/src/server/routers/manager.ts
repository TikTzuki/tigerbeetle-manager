import {z} from "zod";
import {protectedProcedure, publicProcedure, router} from "@/server/trpc";
import {cookies} from "next/headers";
import {getNodeConfigs} from "@/server/nodes";
import {
    getMigrationAccounts,
    getMigrationSyntheticTransfers,
    getNodeBackupConfig,
    getNodeStatus,
    modifyNodeBackupConfig,
    planMigration,
    readNodeAccounts,
    readNodeLsmAccounts,
    readNodeLsmTransfers,
    readNodeTransfers,
    readNodeWalAccounts,
    readNodeWalTransfers,
    startNodeBackup,
    stopNodeBackup,
    triggerNodeBackup,
} from "@/server/grpc-client";

export const managerRouter = router({
    // Login with admin secret.
    login: publicProcedure
        .input(z.object({secretKey: z.string()}))
        .mutation(async ({input}) => {
            if (input.secretKey === process.env.ADMIN_SECRET_KEY) {
                const cookieStore = await cookies();
                cookieStore.set("admin_session", input.secretKey, {
                    httpOnly: true,
                    secure: process.env.NODE_ENV === "production",
                    sameSite: "lax",
                    maxAge: 60 * 60 * 24 * 7,
                });
                return {success: true};
            }
            return {success: false};
        }),

    // Logout.
    logout: publicProcedure.mutation(async () => {
        const cookieStore = await cookies();
        cookieStore.delete("admin_session");
        return {success: true};
    }),

    // Check if authenticated.
    checkAuth: publicProcedure.query(async () => {
        const cookieStore = await cookies();
        const sessionToken = cookieStore.get("admin_session")?.value;
        return {
            isAuthenticated: sessionToken === process.env.ADMIN_SECRET_KEY,
        };
    }),

    // List all discovered clusters (id + node count), derived from GetStatus fan-out.
    getClusters: protectedProcedure.query(async () => {
        const nodes = getNodeConfigs();
        const results = await Promise.allSettled(
            nodes.map(async (node) => {
                try {
                    return await getNodeStatus(node.host, node.port);
                } catch {
                    return null;
                }
            })
        );
        const clusterMap = new Map<string, number>();
        for (const r of results) {
            if (r.status === "fulfilled" && r.value?.cluster_id) {
                const cid = r.value.cluster_id;
                clusterMap.set(cid, (clusterMap.get(cid) ?? 0) + 1);
            }
        }
        return Array.from(clusterMap.entries()).map(([id, nodeCount]) => ({id, nodeCount}));
    }),

    // Get status of all configured nodes (fan-out gRPC calls).
    // Clusters are derived client-side by grouping on status.cluster_id.
    getAllNodeStatuses: protectedProcedure.query(async () => {
        const nodes = getNodeConfigs();
        const results = await Promise.allSettled(
            nodes.map(async (node) => {
                try {
                    const status = await getNodeStatus(node.host, node.port);
                    return {...node, status, online: true as const};
                } catch {
                    return {...node, status: null, online: false as const};
                }
            })
        );
        return results.map((r) =>
            r.status === "fulfilled" ? r.value : {
                id: "unknown",
                host: "",
                port: 0,
                status: null,
                online: false as const,
            }
        );
    }),

    // Fan-out to all nodes and return TigerBeetle addresses for a specific cluster.
    getClusterForMigration: protectedProcedure
        .input(z.object({clusterId: z.string()}))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const results = await Promise.allSettled(
                nodes.map(async (node) => {
                    const status = await getNodeStatus(node.host, node.port);
                    if (status.cluster_id !== input.clusterId) return null;
                    const tbPort = status.process?.address;
                    return tbPort ? `${node.host}:${tbPort}` : null;
                })
            );

            const addresses = results
                .filter((r): r is PromiseFulfilledResult<string> =>
                    r.status === "fulfilled" && r.value !== null
                )
                .map((r) => r.value);

            const clusterNodeCount = results.filter(
                (r) => r.status === "fulfilled" && r.value !== null
            ).length;

            return {
                addresses: addresses.join(","),
                nodeCount: clusterNodeCount,
                onlineCount: addresses.length,
            };
        }),

    // Get status of a single node.
    getNodeStatus: protectedProcedure
        .input(z.object({nodeId: z.string()}))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);

            try {
                const status = await getNodeStatus(node.host, node.port);
                return {...node, status, online: true};
            } catch {
                return {...node, status: null, online: false};
            }
        }),

    // Start backup on a specific node.
    startBackup: protectedProcedure
        .input(z.object({nodeId: z.string(), cronSchedule: z.string()}))
        .mutation(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return startNodeBackup(node.host, node.port, input.cronSchedule);
        }),

    // Stop backup on a specific node.
    stopBackup: protectedProcedure
        .input(z.object({nodeId: z.string()}))
        .mutation(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return stopNodeBackup(node.host, node.port);
        }),

    // Trigger immediate backup on a specific node.
    triggerBackup: protectedProcedure
        .input(z.object({nodeId: z.string()}))
        .mutation(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return triggerNodeBackup(node.host, node.port);
        }),

    // Start backup on ALL nodes at once.
    startBackupAll: protectedProcedure
        .input(z.object({cronSchedule: z.string()}))
        .mutation(async ({input}) => {
            const nodes = getNodeConfigs();
            const results = await Promise.allSettled(
                nodes.map((node) =>
                    startNodeBackup(node.host, node.port, input.cronSchedule)
                )
            );
            const succeeded = results.filter((r) => r.status === "fulfilled").length;
            return {
                success: succeeded > 0,
                message: `Started backups on ${succeeded}/${nodes.length} nodes`,
            };
        }),

    // Stop backup on ALL nodes at once.
    stopBackupAll: protectedProcedure.mutation(async () => {
        const nodes = getNodeConfigs();
        const results = await Promise.allSettled(
            nodes.map((node) => stopNodeBackup(node.host, node.port))
        );
        const succeeded = results.filter((r) => r.status === "fulfilled").length;
        return {
            success: succeeded > 0,
            message: `Stopped backups on ${succeeded}/${nodes.length} nodes`,
        };
    }),

    // Get the current backup config (AWS credentials/endpoint) from a node.
    getBackupConfig: protectedProcedure
        .input(z.object({nodeId: z.string()}))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return getNodeBackupConfig(node.host, node.port);
        }),

    // Update the backup config (AWS credentials/endpoint) on a node.
    modifyBackupConfig: protectedProcedure
        .input(
            z.object({
                nodeId: z.string(),
                awsEndpointUrl: z.string().optional(),
                awsAccessKeyId: z.string().optional(),
                awsSecretAccessKey: z.string().optional(),
                awsDefaultRegion: z.string().optional(),
                awsRequestChecksumCalculation: z.string().optional(),
                awsResponseChecksumValidation: z.string().optional(),
                bucket: z.string().optional(),
                backupFile: z.string().optional(),
            })
        )
        .mutation(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return modifyNodeBackupConfig(node.host, node.port, {
                aws_endpoint_url: input.awsEndpointUrl,
                aws_access_key_id: input.awsAccessKeyId,
                aws_secret_access_key: input.awsSecretAccessKey,
                aws_default_region: input.awsDefaultRegion,
                aws_request_checksum_calculation: input.awsRequestChecksumCalculation,
                aws_response_checksum_validation: input.awsResponseChecksumValidation,
                bucket: input.bucket,
                backup_file: input.backupFile,
            });
        }),

    // Read a page of accounts from a node's data file.
    readAccounts: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            page: z.number().int().min(0).default(0),
            limit: z.number().int().min(1).max(500).default(50),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return readNodeAccounts(node.host, node.port, input.page, input.limit);
        }),

    // Read a page of transfers from a node's data file.
    readTransfers: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            page: z.number().int().min(0).default(0),
            limit: z.number().int().min(1).max(500).default(50),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return readNodeTransfers(node.host, node.port, input.page, input.limit);
        }),

    // Read checkpointed accounts from the LSM (current balances).
    readLsmAccounts: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            page: z.number().int().min(0).default(0),
            limit: z.number().int().min(1).max(500).default(50),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return readNodeLsmAccounts(node.host, node.port, input.page, input.limit);
        }),

    // Read checkpointed transfers from the LSM.
    readLsmTransfers: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            page: z.number().int().min(0).default(0),
            limit: z.number().int().min(1).max(500).default(50),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return readNodeLsmTransfers(node.host, node.port, input.page, input.limit);
        }),

    // Read pre-checkpoint accounts from the WAL (initial balance values).
    readWalAccounts: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            page: z.number().int().min(0).default(0),
            limit: z.number().int().min(1).max(500).default(50),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return readNodeWalAccounts(node.host, node.port, input.page, input.limit);
        }),

    // Read pre-checkpoint transfers from the WAL.
    readWalTransfers: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            page: z.number().int().min(0).default(0),
            limit: z.number().int().min(1).max(500).default(50),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return readNodeWalTransfers(node.host, node.port, input.page, input.limit);
        }),

    // --- Migration ---

    // Pre-flight migration check on a specific node (read-only).
    planMigration: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            cutoffTs: z.string().optional(),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return planMigration(node.host, node.port, input.cutoffTs);
        }),

    // Browse cached migration accounts with filtering and pagination.
    getMigrationAccounts: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            page: z.number().int().min(0).default(0),
            limit: z.number().int().min(1).max(500).default(100),
            filter: z.object({
                id: z.string().optional(),
                ledger: z.number().int().optional(),
                code: z.number().int().optional(),
                flags: z.number().int().optional(),
                user_data_32: z.number().int().optional(),
                user_data_64: z.string().optional(),
                user_data_128: z.string().optional(),
            }).optional(),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return getMigrationAccounts(node.host, node.port, input.page, input.limit, input.filter);
        }),

    // Browse cached migration synthetic transfers with pagination.
    getMigrationSyntheticTransfers: protectedProcedure
        .input(z.object({
            nodeId: z.string(),
            page: z.number().int().min(0).default(0),
            limit: z.number().int().min(1).max(500).default(100),
            ledger: z.number().int().optional(),
        }))
        .query(async ({input}) => {
            const nodes = getNodeConfigs();
            const node = nodes.find((n) => n.id === input.nodeId);
            if (!node) throw new Error(`Node ${input.nodeId} not found`);
            return getMigrationSyntheticTransfers(node.host, node.port, input.page, input.limit, input.ledger);
        }),
});
