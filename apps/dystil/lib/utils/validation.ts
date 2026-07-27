
import { z } from "zod";
import { SettingsStore, User, Credits } from "./tauri";

// Extended settings type that includes fields not yet in generated SettingsStore
type ExtendedSettingsKeys = keyof SettingsStore | "ignoredUrls" | "deviceId" | "updateChannel";

// Zod schemas for validation
export const creditsSchema = z.object({
  amount: z.number().min(0, "Credits amount cannot be negative"),
});

export const userSchema = z.object({
  id: z.string().nullable(),
  name: z.string().nullable(),
  email: z.string().email("Invalid email format").nullable(),
  image: z.string().url("Invalid image URL").nullable(),
  token: z.string().nullable(),
  api_key: z.string().nullable(),
  credits: creditsSchema.nullable(),
  bio: z.string().nullable(),
  website: z.string().url("Invalid website URL").nullable(),
  contact: z.string().nullable(),
  credits_balance: z.number().nullable(),
});

export const settingsStoreSchema = z.object({
  openaiApiKey: z.string(),
  aiModel: z.string().min(1, "AI model is required"),
  customPrompt: z.string().min(10, "Custom prompt must be at least 10 characters"),
  aiUrl: z.string().url("Invalid AI URL format"),
  aiMaxContextChars: z.number().int().min(1000).max(2000000),
  
  // Video Settings
  ocrEngine: z.string().min(1, "OCR engine is required"),
  monitorIds: z.array(z.string()),
  ignoredWindows: z.array(z.string()),
  includedWindows: z.array(z.string()),
  ignoredUrls: z.array(z.string()),
  disableVision: z.boolean(),
  useAllMonitors: z.boolean(),
  fps: z.number().min(0.1, "FPS must be at least 0.1").max(60, "FPS cannot exceed 60"),

  // System Settings
  dataDir: z.string().min(1, "Data directory is required"),
  port: z.number().int().min(1024, "Port must be at least 1024").max(65535, "Port cannot exceed 65535"),
  analyticsEnabled: z.boolean(),
  useChineseMirror: z.boolean(),
  usePiiRemoval: z.boolean(),
  devMode: z.boolean(),
  autoStartEnabled: z.boolean(),
  platform: z.string(),
  
  // Shortcuts
  disabledShortcuts: z.array(z.string()),
  showDystilShortcut: z.string(),
  startRecordingShortcut: z.string(),
  stopRecordingShortcut: z.string(),
  pipeShortcuts: z.record(z.string()),
  showShortcutOverlay: z.boolean().optional(),
  
  // Other
  isLoading: z.boolean(),
  installedPipes: z.array(z.any()), // Define proper pipe schema if needed
  userId: z.string(),
  analyticsId: z.string(),
  user: userSchema,
});

// Validation results
export interface ValidationResult {
  isValid: boolean;
  errors: Record<string, string[]>;
  warnings: Record<string, string[]>;
}

export interface FieldValidationResult {
  isValid: boolean;
  error?: string;
  warning?: string;
}

// Field-specific validators
export const validateField = (
  field: keyof SettingsStore,
  value: any
): FieldValidationResult => {
  try {
    const schemaShape = settingsStoreSchema.shape as Record<string, z.ZodTypeAny>;
    const fieldSchema = schemaShape[field];
    if (!fieldSchema) {
      return { isValid: false, error: "Unknown field" };
    }
    
    fieldSchema.parse(value);
    return { isValid: true };
  } catch (error) {
    if (error instanceof z.ZodError) {
      return {
        isValid: false,
        error: error.errors[0]?.message || "Invalid value",
      };
    }
    return { isValid: false, error: "Validation failed" };
  }
};

// Validate entire settings object
export const validateSettings = (settings: Partial<SettingsStore>): ValidationResult => {
  const result: ValidationResult = {
    isValid: true,
    errors: {},
    warnings: {},
  };

  for (const [field, value] of Object.entries(settings)) {
    const fieldResult = validateField(field as keyof SettingsStore, value);
    if (!fieldResult.isValid && fieldResult.error) {
      result.isValid = false;
      result.errors[field] = [fieldResult.error];
    }
    if (fieldResult.warning) {
      result.warnings[field] = [fieldResult.warning];
    }
  }

  return result;
};

// Sanitize input values
export const sanitizeValue = (field: ExtendedSettingsKeys, value: any): any => {
  switch (field) {
    case "port":
      return Math.max(0, parseInt(String(value)) || 0);
    
    case "fps":
      return Math.max(0.1, Math.min(60, parseFloat(String(value)) || 1));
      
    case "dataDir":
      return String(value).trim();
      
    case "ignoredWindows":
    case "includedWindows":
    case "ignoredUrls":
    case "monitorIds":
      return Array.isArray(value) ? value.filter(Boolean) : [];
      
    default:
      return value;
  }
};

// Performance optimization helpers
export const debounce = <T extends (...args: any[]) => any>(
  func: T,
  wait: number
): ((...args: Parameters<T>) => void) => {
  let timeout: NodeJS.Timeout;
  return (...args: Parameters<T>) => {
    clearTimeout(timeout);
    timeout = setTimeout(() => func(...args), wait);
  };
};

export const throttle = <T extends (...args: any[]) => any>(
  func: T,
  limit: number
): ((...args: Parameters<T>) => void) => {
  let inThrottle: boolean;
  return (...args: Parameters<T>) => {
    if (!inThrottle) {
      func(...args);
      inThrottle = true;
      setTimeout(() => (inThrottle = false), limit);
    }
  };
};

// UI feedback helpers
export const getFieldVariant = (field: keyof SettingsStore, settings: any) => {
  const validation = validateField(field, settings[field]);
  if (!validation.isValid) return "destructive";
  if (validation.warning) return "secondary";
  return "default";
};

export const getFieldHelperText = (field: keyof SettingsStore, settings: any) => {
  const validation = validateField(field, settings[field]);
  return validation.error || validation.warning || "";
};

// URL validation
export const validateUrl = (url: string): FieldValidationResult => {
  if (!url.trim()) {
    return { isValid: false, error: "URL is required" };
  }
  
  try {
    new URL(url);
    return { isValid: true };
  } catch {
    return { isValid: false, error: "Please enter a valid URL" };
  }
};

// Context length validation
export const validateContextLength = (length: number, model: string): FieldValidationResult => {
  if (length < 1000) {
    return { isValid: false, error: "Context length must be at least 1,000 characters" };
  }
  
  if (length > 2000000) {
    return { isValid: false, error: "Context length cannot exceed 2,000,000 characters" };
  }
  
  // Model-specific warnings
  if (model.includes("gpt-4") && length > 128000 * 4) {
    return { isValid: true, warning: "This context length exceeds GPT-4's recommended limit" };
  }
  
  if (model.includes("gpt-3.5") && length > 16000 * 4) {
    return { isValid: true, warning: "This context length exceeds GPT-3.5's limit" };
  }
  
  return { isValid: true };
};
