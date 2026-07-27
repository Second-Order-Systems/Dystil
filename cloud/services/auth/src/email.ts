import nodemailer from "nodemailer";
import { render } from "@react-email/render";

import { EmailVerificationEmail } from "./emails/email-verification.tsx";
import { ResetPasswordEmail } from "./emails/reset-password.tsx";

type AuthEmail = {
  to: string;
  subject: string;
  text: string;
  html: string;
};

const smtpHost = process.env.SMTP_HOST;
const smtpPort = process.env.SMTP_PORT;
const smtpUser = process.env.SMTP_USER;
const smtpPass = process.env.SMTP_PASS;
const smtpFrom = process.env.SMTP_FROM;

function createTransporter() {
  const missing = [
    !smtpHost && "SMTP_HOST",
    !smtpUser && "SMTP_USER",
    !smtpPass && "SMTP_PASS",
    !smtpFrom && "SMTP_FROM",
  ].filter(Boolean) as string[];

  if (missing.length > 0) {
    throw new Error(`Missing auth email env vars: ${missing.join(", ")}`);
  }

  console.log("[email] SMTP config:", {
    host: smtpHost,
    port: Number(smtpPort ?? 587),
    user: smtpUser,
    from: smtpFrom,
  });

  return nodemailer.createTransport({
    host: smtpHost,
    port: Number(smtpPort ?? 587),
    secure: false,
    requireTLS: true,
    auth: {
      user: smtpUser,
      pass: smtpPass,
    },
  });
}

export async function sendAuthEmail(email: AuthEmail) {
  console.log("[email] sendAuthEmail called →", {
    to: email.to,
    subject: email.subject,
  });
  try {
    const transporter = createTransporter();
    const info = await transporter.sendMail({
      from: smtpFrom,
      to: email.to,
      subject: email.subject,
      text: email.text,
      html: email.html,
    });
    console.log("[email] sendMail success:", info.messageId);
  } catch (err) {
    console.error("[email] sendMail FAILED:", err);
    throw err;
  }
}

export async function verificationEmailHtml(url: string, email?: string): Promise<string> {
  return render(
    EmailVerificationEmail({
      url,
      email,
      expirationMinutes: 60,
    })
  );
}

export async function resetPasswordEmailHtml(url: string, email?: string): Promise<string> {
  return render(
    ResetPasswordEmail({
      url,
      email,
      expirationMinutes: 60,
    })
  );
}
