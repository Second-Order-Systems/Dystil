
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
import { Eye, EyeOff } from "lucide-react";

export type SignUpFormProps = {
  className?: string;
  isPending: boolean;
  onSubmit: (name: string, email: string, password: string) => void;
  emailError?: string | null;
  onEmailChange?: () => void;
  onSignIn?: () => void;
  socialPosition?: "top" | "bottom";
  socialButtons?: React.ReactNode;
};

export function SignUpForm({
  className,
  isPending,
  onSubmit,
  emailError,
  onEmailChange,
  onSignIn,
  socialPosition = "bottom",
  socialButtons,
}: SignUpFormProps) {
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");

  const [isPasswordVisible, setIsPasswordVisible] = useState(false);
  const [isConfirmPasswordVisible, setIsConfirmPasswordVisible] =
    useState(false);

  const [fieldErrors, setFieldErrors] = useState<{
    name?: string;
    email?: string;
    password?: string;
    confirmPassword?: string;
  }>({});

  const handleSubmit = (e: SyntheticEvent<HTMLFormElement>) => {
    e.preventDefault();

    const formData = new FormData(e.currentTarget);
    const name = (formData.get("name") as string | null) ?? "";
    const email = formData.get("email") as string;

    if (password !== confirmPassword) {
      setFieldErrors((prev) => ({
        ...prev,
        confirmPassword: "Passwords do not match",
      }));
      setPassword("");
      setConfirmPassword("");
      return;
    }

    onSubmit(name, email, password);
  };

  const showSeparator = socialPosition === "bottom" && socialButtons;
  const displayedEmailError = fieldErrors.email ?? emailError;

  return (
    <Card className={cn("w-full max-w-sm rounded-3xl", className)}>
      <CardHeader>
        <CardTitle className="text-xl font-semibold">Create account</CardTitle>
      </CardHeader>

      <CardContent>
        <div className="flex flex-col gap-6">
          {socialPosition === "top" && (
          <>
            {socialButtons}
 
            {showSeparator && (
                <FieldSeparator className="*:data-[slot=field-separator-content]:bg-card text-xs flex items-center">
                or
              </FieldSeparator>
            )}
          </>
          )}


          <form onSubmit={handleSubmit}>
            <FieldGroup>
              <Field data-invalid={!!fieldErrors.name}>
                <Label htmlFor="name">Name</Label>

                <Input
                  id="name"
                  name="name"
                  type="text"
                  autoComplete="name"
                  placeholder="Your name"
                  required
                  disabled={isPending}
                  onChange={() => {
                    setFieldErrors((prev) => ({
                      ...prev,
                      name: undefined,
                    }));
                  }}
                  onInvalid={(e) => {
                    e.preventDefault();

                    setFieldErrors((prev) => ({
                      ...prev,
                      name: "This field is required",
                    }));
                  }}
                  aria-invalid={!!fieldErrors.name}
                />

                <FieldError>{fieldErrors.name}</FieldError>
              </Field>

              <Field data-invalid={!!displayedEmailError}>
                <Label htmlFor="email">Email</Label>

                <Input
                  id="email"
                  name="email"
                  type="email"
                  autoComplete="email"
                  placeholder="name@example.com"
                  required
                  disabled={isPending}
                  className={cn(
                    displayedEmailError &&
                      "border-destructive focus-visible:ring-destructive",
                  )}
                  onChange={() => {
                    setFieldErrors((prev) => ({
                      ...prev,
                      email: undefined,
                    }));
                    onEmailChange?.();
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
                  aria-invalid={!!displayedEmailError}
                />

                <FieldError>{displayedEmailError}</FieldError>
              </Field>

              <Field data-invalid={!!fieldErrors.password}>
                <Label htmlFor="password">Password</Label>

                <div className="relative">
                  <Input
                    id="password"
                    name="password"
                    type={isPasswordVisible ? "text" : "password"}
                    autoComplete="new-password"
                    value={password}
                    onChange={(e) => {
                      setPassword(e.target.value);
                      setFieldErrors((prev) => ({
                        ...prev,
                        password: undefined,
                      }));
                    }}
                    placeholder="Create a password"
                    required
                    minLength={8}
                    maxLength={128}
                    disabled={isPending}
                    className="pr-11"
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
                  <button
                    type="button"
                    aria-label={isPasswordVisible ? "Hide password" : "Show password"}
                    aria-pressed={isPasswordVisible}
                    disabled={isPending}
                    onClick={() => setIsPasswordVisible((visible) => !visible)}
                    className="absolute inset-y-0 right-0 grid w-11 place-items-center rounded-r-md text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {isPasswordVisible ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                  </button>
                </div>

                <FieldError>{fieldErrors.password}</FieldError>
              </Field>

              <Field data-invalid={!!fieldErrors.confirmPassword}>
                <Label htmlFor="confirmPassword">Confirm password</Label>

                <div className="relative">
                  <Input
                    id="confirmPassword"
                    name="confirmPassword"
                    type={isConfirmPasswordVisible ? "text" : "password"}
                    autoComplete="new-password"
                    value={confirmPassword}
                    onChange={(e) => {
                      setConfirmPassword(e.target.value);

                      setFieldErrors((prev) => ({
                        ...prev,
                        confirmPassword: undefined,
                      }));
                    }}
                    placeholder="Confirm your password"
                    required
                    minLength={8}
                    maxLength={128}
                    disabled={isPending}
                    className="pr-11"
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
                        confirmPassword: msg,
                      }));
                    }}
                    aria-invalid={!!fieldErrors.confirmPassword}
                  />
                  <button
                    type="button"
                    aria-label={isConfirmPasswordVisible ? "Hide confirm password" : "Show confirm password"}
                    aria-pressed={isConfirmPasswordVisible}
                    disabled={isPending}
                    onClick={() => setIsConfirmPasswordVisible((visible) => !visible)}
                    className="absolute inset-y-0 right-0 grid w-11 place-items-center rounded-r-md text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {isConfirmPasswordVisible ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                  </button>
                </div>

                <FieldError>{fieldErrors.confirmPassword}</FieldError>
              </Field>

              <div className="flex flex-col gap-3">
                <Button type="submit" className="rounded-full" disabled={isPending}>
                  {isPending && <Spinner />}

                  Create account
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

        {onSignIn && (
          <div className="flex flex-col gap-3 items-center w-full mt-4">
            <FieldDescription className="text-center">
              Already have an account?{" "}
              <button
                type="button"
                onClick={onSignIn}
                className="underline underline-offset-4"
              >
                Sign in
              </button>
            </FieldDescription>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
