import type {Metadata} from "next";
import {TRPCProvider} from "@/trpc/provider";
import "./globals.css";

export const metadata: Metadata = {
    title: "TigerBeetle Manager",
    description: "Manage TigerBeetle backups with cron scheduling",
};

export default function RootLayout({
                                       children,
                                   }: {
    children: React.ReactNode;
}) {
    return (
        <html lang="en">
        <body className="antialiased">
        <TRPCProvider>{children}</TRPCProvider>
        </body>
        </html>
    );
}
