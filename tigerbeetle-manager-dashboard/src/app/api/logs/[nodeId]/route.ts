import {NextRequest} from "next/server";
import {getNodeConfigs} from "@/server/nodes";
import {LogEntry, streamLogs} from "@/server/grpc-client";
import {cookies} from "next/headers";

export async function GET(
    request: NextRequest,
    {params}: { params: Promise<{ nodeId: string }> }
) {
    // Check authentication
    const cookieStore = await cookies();
    const sessionToken = cookieStore.get("admin_session")?.value;
    if (sessionToken !== process.env.ADMIN_SECRET_KEY) {
        return new Response("Unauthorized", {status: 401});
    }

    const {nodeId} = await params;
    const nodes = getNodeConfigs();
    const node = nodes.find((n) => n.id === nodeId);

    if (!node) {
        return new Response("Node not found", {status: 404});
    }

    // Parse tail parameter from query string
    const url = new URL(request.url);
    const tail = parseInt(url.searchParams.get("tail") || "100", 10);

    // Set up Server-Sent Events stream
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
        start(controller) {
            // Set up gRPC stream
            const cleanup = streamLogs({
                host: node.host,
                port: node.port,
                tail,
                onLog: (entry: LogEntry) => {
                    const data = `data: ${JSON.stringify(entry)}\n\n`;
                    controller.enqueue(encoder.encode(data));
                },
                onError: (err: Error) => {
                    const errorData = `data: ${JSON.stringify({
                        error: err.message,
                        timestamp: new Date().toISOString(),
                    })}\n\n`;
                    controller.enqueue(encoder.encode(errorData));
                },
                onEnd: () => {
                    controller.close();
                },
            });

            // Cleanup on client disconnect
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
