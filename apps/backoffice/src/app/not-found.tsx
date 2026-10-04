import Link from "next/link";

export default function NotFound() {
  return (
    <div className="mx-auto mt-24 max-w-md text-center">
      <h1 className="text-lg font-semibold">404</h1>
      <Link href="/" className="text-sm text-primary underline">Dashboard</Link>
    </div>
  );
}
