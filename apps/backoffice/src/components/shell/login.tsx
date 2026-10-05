"use client";
import * as React from "react";
import { KeyRound } from "lucide-react";
import { Button, Card, CardContent, FieldError, Input, Label, Select } from "@/components/ui/primitives";
import { accessAuthUi, apiUrl, decodeToken, devAuthUi, fetchAccessToken, fetchDevToken, setToken } from "@/lib/auth";
import { useT } from "@/lib/hooks";
import { ROLES, type Role } from "@/lib/rbac";

/** Live mode without a valid token: paste a JWT, or (dev) mint one via core-engine. */
export function LoginScreen() {
  const t = useT();
  const [token, setTok] = React.useState("");
  const [error, setError] = React.useState<string | undefined>();
  const [role, setRole] = React.useState<Role>("admin");
  const [name, setName] = React.useState("");
  const [busy, setBusy] = React.useState(false);

  const signIn = (e: React.FormEvent) => {
    e.preventDefault();
    const v = token.trim();
    if (!decodeToken(v)) return setError(t("auth.invalid"));
    setError(undefined);
    setToken(v);
  };

  const access = async () => {
    setBusy(true);
    try {
      setToken(await fetchAccessToken(apiUrl()!));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  // Behind Cloudflare Access the user is already authenticated: sign in silently.
  React.useEffect(() => {
    if (!accessAuthUi()) return;
    let live = true;
    fetchAccessToken(apiUrl()!)
      .then(setToken)
      .catch((e: unknown) => {
        if (live) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      live = false;
    };
  }, []);

  const dev = async () => {
    setBusy(true);
    try {
      setToken(await fetchDevToken(apiUrl()!, role, name.trim() || undefined));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mx-auto mt-16 grid max-w-lg gap-4" data-testid="login">
      <div className="flex items-center gap-2">
        <KeyRound className="h-5 w-5 text-primary" />
        <h1 className="text-lg font-semibold">{t("auth.title")}</h1>
      </div>
      <p className="text-sm text-muted-foreground">{t("auth.body")}</p>
      {accessAuthUi() && (
        <Card>
          <CardContent className="grid gap-2 pt-4" data-testid="access-login">
            <Button onClick={() => void access()} disabled={busy} data-testid="access-token">{t("auth.access")}</Button>
          </CardContent>
        </Card>
      )}
      <Card>
        <CardContent className="pt-4">
          <form onSubmit={signIn} className="grid gap-3">
            <Label>
              {t("auth.token")}
              <Input name="token" value={token} onChange={(e) => setTok(e.target.value)} placeholder="eyJhbGciOi…" autoComplete="off" />
            </Label>
            <FieldError msg={error} />
            <Button type="submit">{t("auth.signIn")}</Button>
          </form>
        </CardContent>
      </Card>
      {devAuthUi() && (
        <Card>
          <CardContent className="grid gap-3 pt-4" data-testid="dev-login">
            <p className="text-xs text-muted-foreground">{t("auth.dev")}</p>
            <div className="flex flex-wrap items-end gap-2">
              <Label>
                {t("common.role")}
                <Select value={role} onChange={(e) => setRole(e.target.value as Role)} name="dev-role">
                  {ROLES.map((r) => <option key={r} value={r}>{r}</option>)}
                </Select>
              </Label>
              <Label className="flex-1">
                {t("auth.name")}
                <Input value={name} onChange={(e) => setName(e.target.value)} name="dev-name" placeholder={`dev-${role}`} />
              </Label>
              <Button variant="outline" onClick={() => void dev()} disabled={busy} data-testid="dev-token">{t("auth.devButton")}</Button>
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
