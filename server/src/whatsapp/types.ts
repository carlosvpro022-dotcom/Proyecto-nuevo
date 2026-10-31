// Tipos del payload de webhook de la WhatsApp Business Cloud API.
// Solo se declara lo que esta app consume; Meta añade campos con el tiempo.

export interface WebhookBody {
  object?: string;
  entry?: WebhookEntry[];
}

export interface WebhookEntry {
  id?: string;
  changes?: WebhookChange[];
}

export interface WebhookChange {
  field?: string;
  value?: WebhookValue;
}

export interface WebhookValue {
  messaging_product?: string;
  metadata?: { display_phone_number?: string; phone_number_id?: string };
  contacts?: WebhookContact[];
  messages?: InboundMessage[];
  statuses?: StatusUpdate[];
  errors?: MetaErrorDetail[];
}

export interface WebhookContact {
  wa_id?: string;
  profile?: { name?: string };
}

export interface MediaObject {
  id?: string;
  mime_type?: string;
  sha256?: string;
  caption?: string;
  filename?: string;
  voice?: boolean;
  animated?: boolean;
}

export interface InboundMessage {
  id?: string;
  from?: string;
  timestamp?: string; // epoch en segundos, como string
  type?: string;
  text?: { body?: string };
  image?: MediaObject;
  video?: MediaObject;
  audio?: MediaObject;
  document?: MediaObject;
  sticker?: MediaObject;
  location?: { latitude?: number; longitude?: number; name?: string; address?: string };
  contacts?: unknown[];
  reaction?: { message_id?: string; emoji?: string };
  context?: { id?: string; from?: string; forwarded?: boolean; frequently_forwarded?: boolean };
  button?: { text?: string; payload?: string };
  interactive?: {
    type?: string;
    button_reply?: { id?: string; title?: string };
    list_reply?: { id?: string; title?: string; description?: string };
  };
  order?: {
    catalog_id?: string;
    text?: string;
    product_items?: Array<{
      product_retailer_id?: string;
      quantity?: number;
      item_price?: number;
      currency?: string;
    }>;
  };
  system?: { body?: string; type?: string };
  errors?: MetaErrorDetail[];
}

export interface StatusUpdate {
  id?: string;
  status?: 'sent' | 'delivered' | 'read' | 'failed';
  timestamp?: string;
  recipient_id?: string;
  conversation?: { id?: string; expiration_timestamp?: string; origin?: { type?: string } };
  pricing?: { billable?: boolean; category?: string; pricing_model?: string };
  errors?: MetaErrorDetail[];
}

export interface MetaErrorDetail {
  code?: number;
  title?: string;
  message?: string;
  error_data?: { details?: string };
}

export interface SendResult {
  messaging_product?: string;
  contacts?: Array<{ input?: string; wa_id?: string }>;
  messages?: Array<{ id?: string; message_status?: string }>;
}

export interface TemplateSummary {
  id: string;
  name: string;
  language: string;
  status: string;
  category: string;
  components: unknown[];
}

export interface CatalogProduct {
  id: string;
  retailer_id: string;
  name: string;
  description: string;
  price: string;
  currency: string;
  availability: string;
  image_url: string;
  url: string;
}
