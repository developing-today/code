const EMAIL_ADDRESS = /^[^\s@,;<>"]+@[^\s@,;<>"]+\.[^\s@,;<>"]+$/;
const MAX_ADDRESS = 254;
const MAX_SUBJECT = 200;
const MAX_TEXT = 10000;
const PLACEHOLDER = "{code}";

// Grant: { mode: "template" | "free", from: [addresses], maxRecipients: n }.
export default {
  check(body, grant, config) {
    const { subject, text } = body;
    const from = body.from ?? grant.from[0];
    const recipients = recipientList(body);
    if (!recipients || [subject, text, from].some((value) => typeof value !== "string")) {
      return refuse(400, "to, subject, text and from must be strings");
    }
    if ([subject, from, ...recipients].some((value) => /[\r\n]/.test(value))) {
      return refuse(400, "to, subject and from must be one line");
    }
    if (recipients.length === 0 || recipients.length > grant.maxRecipients) {
      return refuse(400, `to must have 1 to ${grant.maxRecipients} addresses`);
    }
    if (recipients.some((to) => !EMAIL_ADDRESS.test(to) || to.length > MAX_ADDRESS)) {
      return refuse(400, "to must be email addresses");
    }
    if (subject.length > MAX_SUBJECT || text.length > MAX_TEXT) {
      return refuse(400, "subject or text is too long");
    }
    if (!grant.from.includes(from)) {
      return refuse(403, "caller may not send from that address");
    }
    if (grant.mode === "template" && !config.templates.some((template) => matchesTemplate(template, subject, text))) {
      return refuse(400, "template");
    }
    return null;
  },

  rateKeys(body) {
    return recipientList(body).map((to) => to.toLowerCase());
  },

  async deliver(body, grant, env) {
    const from = body.from ?? grant.from[0];
    const messageIds = [];
    for (const to of recipientList(body)) {
      const sent = await env.EMAIL.send({ from, to, subject: body.subject, text: body.text });
      messageIds.push(sent.messageId);
    }
    return { messageIds };
  },
};

function recipientList({ to }) {
  if (typeof to === "string") {
    return [to];
  }
  if (Array.isArray(to) && to.every((value) => typeof value === "string")) {
    return to;
  }
  return null;
}

function matchesTemplate({ subject, body }, actualSubject, actualText) {
  if (subject !== actualSubject) {
    return false;
  }
  const pattern = body.split(PLACEHOLDER).map(escapeRegExp).join("\\d{6}");
  return new RegExp(`^${pattern}$`).test(actualText);
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function refuse(status, error) {
  return { status, error };
}
