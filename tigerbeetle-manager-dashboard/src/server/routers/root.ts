import {router} from "@/server/trpc";
import {exampleRouter} from "@/server/routers/example";
import {managerRouter} from "@/server/routers/manager";

export const appRouter = router({
    example: exampleRouter,
    manager: managerRouter,
});

export type AppRouter = typeof appRouter;
