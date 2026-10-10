const TEMPLATE_SUBJECTS = new Set(["id: confirm your address", "id: sign-in code"]);
const TEMPLATE_BODY = /^Your code is \d{6}\. It works for 15 minutes\.\n?$/;
const EMAIL_ADDRESS = /^[^\s@,;<>"]+@[^\s@,;<>"]+\.[^\s@,;<>"]+$/;
const MAX_SUBJECT = 200;
const MAX_TEXT = 10000;

// Grant: { mode: "template" | "free", from: [addresses this caller may send as] }.
export default {
  check({ to, subject, text, from }, grant) {
    const sender = from ?? grant.from[0];
    if ([to, subject, text, sender].some((value) => typeof value !== "string")) {
      return refuse(400, "to, subject, text and from must be strings");
    }
    if ([to, subject, sender].some((value) => /[\r\n]/.test(value))) {
      return refuse(400, "to, subject and from must be one line");
    }
    if (!EMAIL_ADDRESS.test(to) || to.length > 254) {
      return refuse(400, "to must be one email address");
    }
    if (subject.length > MAX_SUBJECT || text.length > MAX_TEXT) {
      return refuse(400, "subject or text is too long");
    }
    if (!grant.from.includes(sender)) {
      return refuse(403, "caller may not send from that address");
    }
    if (grant.mode === "template" && !(TEMPLATE_SUBJECTS.has(subject) && TEMPLATE_BODY.test(text))) {
      return refuse(400, "template");
    }
    return null;
  },

  rateKey({ to }) {
    return to.toLowerCase();
  },

  async deliver({ to, subject, text, from }, grant, env) {
    const sent = await env.EMAIL.send({ from: from ?? grant.from[0], to, subject, text });
    return { messageId: sent.messageId };
  },
};

function refuse(status, error) {
  return { status, error };
}
