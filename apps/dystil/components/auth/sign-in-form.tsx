
"use client";

import { type SyntheticEvent, useState } from "react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldSeparator,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

export type SignInFormProps = {
  className?: string;
  isPending: boolean;
  onSubmit: (email: string, password: string) => void;
  onForgotPassword?: () => void;
  onSignUp?: () => void;
  socialPosition?: "top" | "bottom";
  socialButtons?: React.ReactNode;
};

export function SignInForm({
  className,
  isPending,
  onSubmit,
  onForgotPassword,
  onSignUp,
  socialPosition = "bottom",
  socialButtons,
}: SignInFormProps) {
  const [password, setPassword] = useState("");

  const [fieldErrors, setFieldErrors] = useState<{
    email?: string;
    password?: string;
  }>({});

  const handleSubmit = (e: SyntheticEvent<HTMLFormElement>) => {
    e.preventDefault();

    const formData = new FormData(e.currentTarget);
    const email = formData.get("email") as string;

    onSubmit(email, password);
  };

  const showSeparator = socialPosition === "bottom" && socialButtons;

  return (
    <Card className={cn("w-full max-w-sm rounded-3xl", className)}>
      <CardHeader>
        <CardTitle className="text-xl font-semibold">Sign in</CardTitle>
      </CardHeader>

      <CardContent>
        <div className="flex flex-col gap-6">
          {socialPosition === "top" && (
            <>
            {socialButtons}
            {showSeparator && (
            <FieldSeparator className="*:data-[slot=field-separator-content]:bg-card m-0 text-xs flex items-center">
              or
            </FieldSeparator>
             )}
            </>
          )}

          <form onSubmit={handleSubmit}>
            <FieldGroup>
              <Field data-invalid={!!fieldErrors.email}>
                <Label htmlFor="email">Email</Label>

                <Input
                  id="email"
                  name="email"
                  type="email"
                  autoComplete="email"
                  placeholder="name@example.com"
                  required
                  disabled={isPending}
                  onChange={() => {
                    setFieldErrors((prev) => ({
                      ...prev,
                      email: undefined,
                    }));
                  }}
                  onInvalid={(e) => {
                    e.preventDefault();
                    const el = e.target as HTMLInputElement;
                    const msg = el.validity.valueMissing
                      ? "This field is required"
                      : "Please enter a valid email address";

                    setFieldErrors((prev) => ({
                      ...prev,
                      email: msg,
                    }));
                  }}
                  aria-invalid={!!fieldErrors.email}
                />

                <FieldError>{fieldErrors.email}</FieldError>
              </Field>

              <Field data-invalid={!!fieldErrors.password}>
                <Label htmlFor="password">Password</Label>

                <Input
                  id="password"
                  name="password"
                  type="password"
                  autoComplete="current-password"
                  value={password}
                  onChange={(e) => {
                    setPassword(e.target.value);

                    setFieldErrors((prev) => ({
                      ...prev,
                      password: undefined,
                    }));
                  }}
                  placeholder="Enter your password"
                  required
                  minLength={8}
                  maxLength={128}
                  disabled={isPending}
                  onInvalid={(e) => {
                    e.preventDefault();
                    const el = e.target as HTMLInputElement;
                    const msg = el.validity.valueMissing
                      ? "This field is required"
                      : el.validity.tooShort
                        ? "Password must be at least 8 characters"
                        : "Password is too long";

                    setFieldErrors((prev) => ({
                      ...prev,
                      password: msg,
                    }));
                  }}
                  aria-invalid={!!fieldErrors.password}
                />

                <FieldError>{fieldErrors.password}</FieldError>
              </Field>

              <div className="flex flex-col gap-3">
                <Button type="submit" className="rounded-full" disabled={isPending}>
                  {isPending && <Spinner />}

                  Sign in
                </Button>
              </div>
            </FieldGroup>
          </form>

          {socialPosition === "bottom" && (
            <>
              {showSeparator && (
                <FieldSeparator className="*:data-[slot=field-separator-content]:bg-card text-xs flex items-center">
                  or
                </FieldSeparator>
              )}

              {socialButtons}
            </>
          )}
        </div>

        <div className="flex flex-col gap-3 items-center w-full mt-4">
          {onForgotPassword && (
            <button
              type="button"
              onClick={onForgotPassword}
              className="self-center text-sm underline-offset-4 hover:underline text-muted-foreground"
            >
              Forgot password?
            </button>
          )}

          {onSignUp && (
            <FieldDescription className="text-center">
              Don&apos;t have an account?{" "}
              <button
                type="button"
                onClick={onSignUp}
                className="underline underline-offset-4"
              >
                Sign up
              </button>
            </FieldDescription>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
