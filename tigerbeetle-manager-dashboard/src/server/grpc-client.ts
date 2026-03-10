// gRPC client for connecting to manager nodes.

import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
import path from "path";

const PROTO_PATH = path.resolve(
    process.cwd(),
    "../proto/manager.proto"
);

const packageDefinition = protoLoader.loadSync(PROTO_PATH, {
    keepCase: true,
    longs: String,
    enums: String,
    defaults: true,
    oneofs: true,
});

const proto = grpc.loadPackageDefinition(packageDefinition) as any;

export interface ProcessStatus {
    state: string;
    pid: number;
    exe: string;
    args: string[];
    data_file: string;
    address: string;
}

export interface BackupStatus {
    enabled: boolean;
    cron_schedule: string;
    bucket: string;
    last_backup_at: string;
    last_error: string;
}

export interface NodeStatus {
    node_id: string;
    process: ProcessStatus;
    backup: BackupStatus;
    uptime_seconds: string;
}

export interface GrpcResponse {
    success: boolean;
    message: string;
}

function createClient(address: string) {
    return new proto.tigerbeetle.manager.ManagerNode(
        address,
        grpc.credentials.createInsecure()
    );
}

function promisify<T>(
    client: any,
    method: string,
    request: any
): Promise<T> {
    return new Promise((resolve, reject) => {
        client[method](request, (err: any, response: T) => {
            if (err) reject(err);
            else resolve(response);
        });
    });
}

export async function getNodeStatus(
    host: string,
    port: number
): Promise<NodeStatus> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify<NodeStatus>(client, "GetStatus", {});
    } finally {
        client.close();
    }
}

export async function startNodeBackup(
    host: string,
    port: number,
    cronSchedule: string
): Promise<GrpcResponse> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify<GrpcResponse>(client, "StartBackup", {
            cron_schedule: cronSchedule,
        });
    } finally {
        client.close();
    }
}

export async function stopNodeBackup(
    host: string,
    port: number
): Promise<GrpcResponse> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify<GrpcResponse>(client, "StopBackup", {});
    } finally {
        client.close();
    }
}

export async function triggerNodeBackup(
    host: string,
    port: number
): Promise<GrpcResponse> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify<GrpcResponse>(client, "TriggerBackup", {});
    } finally {
        client.close();
    }
}

export interface BackupConfig {
    config_file_configured: boolean;
    aws_endpoint_url: string;
    aws_access_key_id: string;
    aws_secret_access_key: string;
    aws_default_region: string;
    aws_request_checksum_calculation: string;
    aws_response_checksum_validation: string;
    bucket: string;
    backup_file: string;
}

export async function getNodeBackupConfig(
    host: string,
    port: number
): Promise<BackupConfig> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify<BackupConfig>(client, "GetBackupConfig", {});
    } finally {
        client.close();
    }
}

export interface ModifyBackupConfigInput {
    aws_endpoint_url?: string;
    aws_access_key_id?: string;
    aws_secret_access_key?: string;
    aws_default_region?: string;
    aws_request_checksum_calculation?: string;
    aws_response_checksum_validation?: string;
    bucket?: string;
    backup_file?: string;
}

export async function modifyNodeBackupConfig(
    host: string,
    port: number,
    config: ModifyBackupConfigInput
): Promise<GrpcResponse> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify<GrpcResponse>(client, "ModifyBackupConfig", {
            aws_endpoint_url: config.aws_endpoint_url ?? "",
            aws_access_key_id: config.aws_access_key_id ?? "",
            aws_secret_access_key: config.aws_secret_access_key ?? "",
            aws_default_region: config.aws_default_region ?? "",
            aws_request_checksum_calculation: config.aws_request_checksum_calculation ?? "",
            aws_response_checksum_validation: config.aws_response_checksum_validation ?? "",
            bucket: config.bucket ?? "",
            backup_file: config.backup_file ?? "",
        });
    } finally {
        client.close();
    }
}

export interface LogEntry {
    timestamp: string;
    level: string;
    message: string;
}

export interface StreamLogsOptions {
    host: string;
    port: number;
    tail?: number;
    onLog: (entry: LogEntry) => void;
    onError?: (err: Error) => void;
    onEnd?: () => void;
}

export function streamLogs(options: StreamLogsOptions): () => void {
    const client = createClient(`${options.host}:${options.port}`);
    const stream = client.StreamLogs({tail: options.tail || 0});

    stream.on("data", (entry: LogEntry) => {
        options.onLog(entry);
    });

    stream.on("error", (err: any) => {
        if (options.onError) {
            options.onError(err);
        }
    });

    stream.on("end", () => {
        if (options.onEnd) {
            options.onEnd();
        }
    });

    // Return cleanup function
    return () => {
        stream.cancel();
        client.close();
    };
}

export interface AccountRecord {
    id: string;
    debits_pending: string;
    debits_posted: string;
    credits_pending: string;
    credits_posted: string;
    user_data_128: string;
    user_data_64: string;
    user_data_32: number;
    ledger: number;
    code: number;
    flags: number;
    timestamp: string;
}

export interface TransferRecord {
    id: string;
    debit_account_id: string;
    credit_account_id: string;
    amount: string;
    pending_id: string;
    user_data_128: string;
    user_data_64: string;
    user_data_32: number;
    timeout: number;
    ledger: number;
    code: number;
    flags: number;
    timestamp: string;
}

export async function readNodeAccounts(
    host: string,
    port: number,
    page: number,
    limit: number
): Promise<{ accounts: AccountRecord[]; page: number; limit: number }> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify(client, "ReadAccounts", {page, limit});
    } finally {
        client.close();
    }
}

export async function readNodeTransfers(
    host: string,
    port: number,
    page: number,
    limit: number
): Promise<{ transfers: TransferRecord[]; page: number; limit: number }> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify(client, "ReadTransfers", {page, limit});
    } finally {
        client.close();
    }
}

export async function readNodeLsmAccounts(
    host: string,
    port: number,
    page: number,
    limit: number
): Promise<{ accounts: AccountRecord[]; page: number; limit: number }> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify(client, "ReadLsmAccounts", {page, limit});
    } finally {
        client.close();
    }
}

export async function readNodeLsmTransfers(
    host: string,
    port: number,
    page: number,
    limit: number
): Promise<{ transfers: TransferRecord[]; page: number; limit: number }> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify(client, "ReadLsmTransfers", {page, limit});
    } finally {
        client.close();
    }
}

export async function readNodeWalAccounts(
    host: string,
    port: number,
    page: number,
    limit: number
): Promise<{ accounts: AccountRecord[]; page: number; limit: number }> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify(client, "ReadWalAccounts", {page, limit});
    } finally {
        client.close();
    }
}

export async function readNodeWalTransfers(
    host: string,
    port: number,
    page: number,
    limit: number
): Promise<{ transfers: TransferRecord[]; page: number; limit: number }> {
    const client = createClient(`${host}:${port}`);
    try {
        return await promisify(client, "ReadWalTransfers", {page, limit});
    } finally {
        client.close();
    }
}
