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

const emailVerificationLocalization = {
  VERIFY_YOUR_EMAIL_ADDRESS: "Verify your email address",
  CLICK_BUTTON_TO_VERIFY_EMAIL:
    "Click the button below to verify your email address for your Dystil account.",
  VERIFY_EMAIL_ADDRESS: "Verify email address",
  OR_COPY_AND_PASTE_URL: "Or copy and paste this URL into your browser:",
  THIS_LINK_EXPIRES_IN_MINUTES: "This link expires in {expirationMinutes} minutes.",
  EMAIL_SENT_BY: "Email sent by Dystil.",
  IF_YOU_DIDNT_REQUEST_THIS_EMAIL:
    "If you didn't request this email, you can safely ignore it. Someone else might have typed your email address by mistake.",
};

export interface EmailVerificationEmailProps {
  url: string;
  email?: string;
  expirationMinutes?: number;
}

export const EmailVerificationEmail = ({
  url,
  email,
  expirationMinutes = 60,
}: EmailVerificationEmailProps) => {
  const localization = emailVerificationLocalization;
  const previewText = localization.VERIFY_YOUR_EMAIL_ADDRESS;

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
                {localization.VERIFY_EMAIL_ADDRESS}
              </Heading>

              <Text className="text-sm font-normal">
                {localization.CLICK_BUTTON_TO_VERIFY_EMAIL}
              </Text>

              {email && (
                <Text className="text-sm font-normal">
                  Verifying:{" "}
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
                  {localization.VERIFY_EMAIL_ADDRESS}
                </Button>
              </Section>

              <Text className="mb-3 text-xs text-muted-foreground">
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
                <Text className="mb-3 text-xs text-muted-foreground">
                  {localization.THIS_LINK_EXPIRES_IN_MINUTES.replace(
                    "{expirationMinutes}",
                    expirationMinutes.toString()
                  )}
                </Text>
              )}

              <Text className="mt-3 text-xs text-muted-foreground">
                {localization.EMAIL_SENT_BY}
              </Text>

              <Text className="mt-3 text-xs text-muted-foreground">
                {localization.IF_YOU_DIDNT_REQUEST_THIS_EMAIL}
              </Text>
            </Section>
          </Container>
        </Body>
      </Tailwind>
    </Html>
  );
};

EmailVerificationEmail.PreviewProps = {
  url: "https://dystil.app/auth/verify-email?token=example-token",
  email: "user@example.com",
  expirationMinutes: 60,
} as EmailVerificationEmailProps;

export default EmailVerificationEmail;
