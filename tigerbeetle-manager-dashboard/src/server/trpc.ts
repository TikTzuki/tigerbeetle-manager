import {initTRPC, TRPCError} from "@trpc/server";
import {cookies} from "next/headers";

interface Context {
    isAuthenticated: boolean;
}

const t = initTRPC.context<Context>().create();

export const router = t.router;
export const publicProcedure = t.procedure;

// Auth middleware
const isAuthed = t.middleware(({ctx, next}) => {
    if (!ctx.isAuthenticated) {
        throw new TRPCError({code: "UNAUTHORIZED"});
    }
    return next({ctx});
});

export const protectedProcedure = t.procedure.use(isAuthed);

// Create context for each request
export async function createContext() {
    const cookieStore = await cookies();
    const sessionToken = cookieStore.get("admin_session")?.value;

    return {
        isAuthenticated: sessionToken === process.env.ADMIN_SECRET_KEY,
    };
}
