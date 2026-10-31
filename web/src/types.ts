export interface Label {
  id: number;
  name: string;
  color: string;
  contact_count?: number;
}

export interface MessageMedia {
  id: string | null;
  state: 'pending' | 'ready' | 'error';
  mime: string | null;
  filename: string | null;
  size: number | null;
  url: string | null;
}

export type MessageStatus = 'pending' | 'sent' | 'delivered' | 'read' | 'failed' | 'received';

export interface Message {
  id: number;
  wamId: string | null;
  conversationId: number;
  waId: string;
  direction: 'in' | 'out';
  type: string;
  body: string | null;
  status: MessageStatus;
  error: { code: string | null; message: string } | null;
  timestamp: number;
  replyTo: string | null;
  reaction: string | null;
  reactionTarget: string | null;
  media: MessageMedia | null;
  payload: unknown;
}

export interface Conversation {
  id: number;
  waId: string;
  name: string;
  profileName: string | null;
  displayName: string | null;
  notes: string | null;
  isBlocked: boolean;
  labels: Label[];
  lastMessageAt: number | null;
  lastInboundAt: number | null;
  unreadCount: number;
  isArchived: boolean;
  isPinned: boolean;
  isMuted: boolean;
  windowOpen: boolean;
  windowClosesAt: number | null;
  lastMessage: Message | null;
}

export interface Contact {
  waId: string;
  name: string;
  displayName: string | null;
  profileName: string | null;
  notes: string | null;
  isBlocked: boolean;
  labels: Label[];
  conversationId: number | null;
}

export interface Template {
  id: string;
  name: string;
  language: string;
  status: string;
  category: string;
  components: Array<{
    type?: string;
    text?: string;
    format?: string;
    buttons?: Array<{ text?: string; type?: string }>;
  }>;
}

export interface CatalogSummary {
  id: string;
  name: string;
  product_count?: number;
}

export interface Product {
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

export interface AutoReplyRule {
  id: number;
  name: string;
  kind: 'welcome' | 'away' | 'keyword';
  enabled: number;
  keyword: string | null;
  match_type: 'contains' | 'equals' | 'starts_with';
  reply_text: string;
  active_from: number | null;
  active_to: number | null;
  priority: number;
}

export interface BroadcastList {
  id: number;
  name: string;
  members: string[];
}

export interface SettingsResponse {
  settings: Record<string, string | null>;
  whatsapp: {
    configured: boolean;
    phoneNumberId: string | null;
    wabaId: string | null;
    graphVersion: string;
    signatureEnforced: boolean;
    verifyTokenSet: boolean;
  };
  warnings: string[];
  realtimeClients: number;
  limitations: string[];
}

export interface Stats {
  conversations: number;
  unread: number;
  contacts: number;
  messagesIn7d: number;
  messagesOut7d: number;
  failed7d: number;
  openWindows: number;
  byLabel: Array<{ name: string; color: string; n: number }>;
}
