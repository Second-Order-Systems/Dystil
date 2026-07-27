import {
  Body,
  Button,
  Container,
  Head,
  Heading,
  Hr,
  Html,
  Link,
  Preview,
  pixelBasedPreset,
  Section,
  Tailwind,
  Text,
} from "@react-email/components";

const resetPasswordEmailLocalization = {
  RESET_YOUR_PASSWORD: "Reset your password",
  WE_RECEIVED_REQUEST_TO_RESET_PASSWORD:
    "We received a request to reset the password for your Dystil account.",
  RESET_PASSWORD: "Reset password",
  OR_COPY_AND_PASTE_URL: "Or copy and paste this URL into your browser:",
  THIS_LINK_EXPIRES_IN_MINUTES: "This link expires in {expirationMinutes} minutes.",
  EMAIL_SENT_BY: "Email sent by Dystil.",
  IF_YOU_DIDNT_REQUEST_PASSWORD_RESET:
    "If you didn't request a password reset, you can safely ignore this email. Your password will remain unchanged.",
};

export interface ResetPasswordEmailProps {
  url: string;
  email?: string;
  expirationMinutes?: number;
}

export const ResetPasswordEmail = ({
  url,
  email,
  expirationMinutes = 60,
}: ResetPasswordEmailProps) => {
  const localization = resetPasswordEmailLocalization;
  const previewText = localization.RESET_YOUR_PASSWORD;

  return (
    <Html>
      <Head>
        <meta content="light dark" name="color-scheme" />
        <meta content="light dark" name="supported-color-schemes" />
      </Head>

      <Preview>{previewText}</Preview>

      <Tailwind config={{ presets: [pixelBasedPreset] }}>
        <Body className="bg-background font-sans">
          <Container className="mx-auto my-auto max-w-xl px-2 py-10">
            <Section className="bg-card text-card-foreground rounded-none border border-border p-8">
              <Heading className="m-0 mb-5 text-2xl font-semibold">
                {localization.RESET_YOUR_PASSWORD}
              </Heading>

              <Text className="text-sm">
                {localization.WE_RECEIVED_REQUEST_TO_RESET_PASSWORD}
              </Text>

              {email && (
                <Text className="text-sm">
                  Account:{" "}
                  <Link
                    href={`mailto:${email}`}
                    className="text-primary font-medium"
                  >
                    {email}
                  </Link>
                </Text>
              )}

              <Section className="my-6">
                <Button
                  href={url}
                  className="inline-block whitespace-nowrap rounded-none text-sm font-medium py-2.5 px-6 bg-primary text-primary-foreground no-underline"
                >
                  {localization.RESET_PASSWORD}
                </Button>
              </Section>

              <Text className="m-0 mb-3 text-xs text-muted-foreground">
                {localization.OR_COPY_AND_PASTE_URL}
              </Text>

              <Link
                className="break-all text-xs text-primary"
                href={url}
              >
                {url}
              </Link>

              <Hr className="my-6 w-full border border-solid border-border" />

              {expirationMinutes && (
                <Text className="m-0 mb-3 text-xs text-muted-foreground">
                  {localization.THIS_LINK_EXPIRES_IN_MINUTES.replace(
                    "{expirationMinutes}",
                    expirationMinutes.toString()
                  )}
                </Text>
              )}

              <Text className="m-0 text-xs text-muted-foreground">
                {localization.EMAIL_SENT_BY}
              </Text>

              <Text className="m-0 text-xs text-muted-foreground">
                {localization.IF_YOU_DIDNT_REQUEST_PASSWORD_RESET}
              </Text>
            </Section>
          </Container>
        </Body>
      </Tailwind>
    </Html>
  );
};

ResetPasswordEmail.PreviewProps = {
  url: "https://dystil.app/auth/reset-password?token=example-token",
  email: "user@example.com",
  expirationMinutes: 60,
} as ResetPasswordEmailProps;

export default ResetPasswordEmail;
