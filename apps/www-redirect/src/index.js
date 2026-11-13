export default {
  fetch(request) {
    const url = new URL(request.url);
    url.hostname = "zend.invalid";
    return Response.redirect(url.toString(), 301);
  },
};
