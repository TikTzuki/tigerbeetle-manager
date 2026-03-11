import {NextRequest} from "next/server";
import {getNodeConfigs} from "@/server/nodes";
import {executeMigration, MigrationProgress} from "@/server/grpc-client";
import {cookies} from "next/headers";

export async function POST(request: NextRequest) {
    // Check authentication
    const cookieStore = await cookies();
    const sessionToken = cookieStore.get("admin_session")?.value;
    if (sessionToken !== process.env.ADMIN_SECRET_KEY) {
        return new Response("Unauthorized", {status: 401});
    }

    const body = await request.json();
    const {nodeId, newClusterId, newAddresses} = body as {
        nodeId: string;
        newClusterId: number;
        newAddresses: string;
    };

    if (!nodeId || newClusterId == null || !newAddresses) {
        return new Response("Missing required fields: nodeId, newClusterId, newAddresses", {
            status: 400,
        });
    }

    const nodes = getNodeConfigs();
    const node = nodes.find((n) => n.id === nodeId);
    if (!node) {
        return new Response("Node not found", {status: 404});
    }

    const encoder = new TextEncoder();
    const stream = new ReadableStream({
        start(controller) {
            const cleanup = executeMigration({
                host: node.host,
                port: node.port,
                newClusterId,
                newAddresses,
                onProgress: (progress: MigrationProgress) => {
                    const data = `data: ${JSON.stringify(progress)}\n\n`;
                    controller.enqueue(encoder.encode(data));
                },
                onError: (err: Error) => {
                    const errorData = `data: ${JSON.stringify({
                        phase: "error",
                        imported: "0",
                        total: "0",
                        done: false,
                        error: err.message,
                    })}\n\n`;
                    controller.enqueue(encoder.encode(errorData));
                    controller.close();
                },
                onEnd: () => {
                    controller.close();
                },
            });

            request.signal.addEventListener("abort", () => {
                cleanup();
                controller.close();
            });
        },
    });

    return new Response(stream, {
        headers: {
            "Content-Type": "text/event-stream",
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
        },
    });
}
