import Container from "@mui/material/Container";
import Typography from "@mui/material/Typography";
import Box from "@mui/material/Box";

export default function Home() {
  return (
    <Container maxWidth="lg">
      <Box sx={{ py: 6 }}>
        <Typography variant="h3" component="h1" gutterBottom>
          MPEx
        </Typography>
        <Typography variant="body1" color="text.secondary">
          Mortgage product eligibility &amp; pricing engine — rules admin coming
          soon.
        </Typography>
      </Box>
    </Container>
  );
}
