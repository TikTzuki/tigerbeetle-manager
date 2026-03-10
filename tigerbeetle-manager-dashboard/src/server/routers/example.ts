import {z} from "zod";
import {publicProcedure, router} from "@/server/trpc";

export const exampleRouter = router({
    hello: publicProcedure
        .input(z.object({name: z.string()}))
        .query(({input}) => {
            return {
                greeting: `Hello, ${input.name}!`,
            };
        }),
});
