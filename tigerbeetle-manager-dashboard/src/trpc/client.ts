import {createTRPCReact} from "@trpc/react-query";
import type {AppRouter} from "@/server/routers/root";

export const trpc = createTRPCReact<AppRouter>();
